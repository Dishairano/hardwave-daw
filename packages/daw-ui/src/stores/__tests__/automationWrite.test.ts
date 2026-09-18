import { describe, it, expect, vi, beforeEach } from 'vitest'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useAutomationWriteStore, normalizeVolumeDb, normalizePan } = await import(
  '../automationWriteStore'
)

describe('automation write', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockResolvedValue(null)
    useAutomationWriteStore.setState({ mode: 'off', touching: [] })
  })

  it('records nothing while the mode is off', () => {
    const store = useAutomationWriteStore.getState()
    store.writeSample('t1', { kind: 'track_volume' }, 0.5)
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('opens one session for a drag and closes it on release', () => {
    useAutomationWriteStore.getState().setMode('write')
    const store = useAutomationWriteStore.getState()
    store.writeSample('t1', { kind: 'track_volume' }, 0.4)
    store.writeSample('t1', { kind: 'track_volume' }, 0.5)
    store.writeSample('t1', { kind: 'track_volume' }, 0.6)

    const begins = invokeMock.mock.calls.filter(c => c[0] === 'automation_touch_begin')
    const samples = invokeMock.mock.calls.filter(c => c[0] === 'automation_write_sample')
    expect(begins).toHaveLength(1)
    expect(samples).toHaveLength(3)

    store.endTouch('t1', { kind: 'track_volume' })
    expect(invokeMock.mock.calls.filter(c => c[0] === 'automation_touch_end')).toHaveLength(1)
  })

  it('keeps two controls in separate sessions', () => {
    useAutomationWriteStore.getState().setMode('touch')
    const store = useAutomationWriteStore.getState()
    store.writeSample('t1', { kind: 'track_volume' }, 0.4)
    store.writeSample('t1', { kind: 'track_pan' }, 0.9)
    expect(invokeMock.mock.calls.filter(c => c[0] === 'automation_touch_begin')).toHaveLength(2)
  })

  it('a release without a gesture does nothing', () => {
    useAutomationWriteStore.getState().setMode('write')
    invokeMock.mockClear()
    useAutomationWriteStore.getState().endTouch('t1', { kind: 'track_volume' })
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('cycles off, write, touch, latch and back', () => {
    const store = () => useAutomationWriteStore.getState()
    expect(store().mode).toBe('off')
    store().cycleMode(); expect(store().mode).toBe('write')
    store().cycleMode(); expect(store().mode).toBe('touch')
    store().cycleMode(); expect(store().mode).toBe('latch')
    store().cycleMode(); expect(store().mode).toBe('off')
  })

  it('normalizes a fader onto the range the engine maps a lane to', () => {
    // The engine maps a volume lane across -60 dB to +6 dB.
    expect(normalizeVolumeDb(-60)).toBe(0)
    expect(normalizeVolumeDb(6)).toBe(1)
    expect(normalizeVolumeDb(-27)).toBeCloseTo(0.5, 2)
    expect(normalizeVolumeDb(99)).toBe(1)
    expect(normalizePan(-1)).toBe(0)
    expect(normalizePan(0)).toBe(0.5)
    expect(normalizePan(1)).toBe(1)
  })
})
