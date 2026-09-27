import { describe, it, expect, vi, beforeEach } from 'vitest'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }))

const { useMidiCcStore, BUILT_IN_CC_LANES, CC_LANE_RESOLUTION, laneDefinition } = await import(
  '../midiCcStore'
)

const MOD = BUILT_IN_CC_LANES.find(l => l.id === 'cc1')!
const BEND = BUILT_IN_CC_LANES.find(l => l.id === 'pb')!
const LENGTH = 3840

describe('controller lanes read and write the clip', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockResolvedValue([])
    useMidiCcStore.setState({ values: {}, visibleLanes: {}, laneHeight: {} })
  })

  it('holds a point until the next one, the way a controller behaves', async () => {
    invokeMock.mockResolvedValueOnce([
      { tick: 0, value: 0 },
      { tick: LENGTH / 2, value: 1 },
    ])
    await useMidiCcStore.getState().loadLane('t1', 'c1', MOD, LENGTH)
    const arr = useMidiCcStore.getState().getValues('c1', 'cc1')
    expect(arr).toHaveLength(CC_LANE_RESOLUTION)
    expect(arr[0]).toBe(0)
    expect(arr[Math.floor(CC_LANE_RESOLUTION / 4)]).toBe(0)
    expect(arr[CC_LANE_RESOLUTION - 1]).toBe(1)
  })

  it('reads pitch bend centred, so the lane draws it in the middle', async () => {
    invokeMock.mockResolvedValueOnce([{ tick: 0, value: 0 }])
    await useMidiCcStore.getState().loadLane('t1', 'c1', BEND, LENGTH)
    expect(useMidiCcStore.getState().getValues('c1', 'pb')[0]).toBe(0.5)
  })

  it('writes back only where the value changes', async () => {
    const store = useMidiCcStore.getState()
    store.setValueAt('c1', MOD, 100, 0.5)
    store.setValueAt('c1', MOD, 101, 0.5)
    store.setValueAt('c1', MOD, 200, 1)
    invokeMock.mockReset()
    invokeMock.mockResolvedValue(null)
    await useMidiCcStore.getState().flushLane('t1', 'c1', MOD, LENGTH)

    expect(invokeMock).toHaveBeenCalledTimes(1)
    const [command, args] = invokeMock.mock.calls[0] as [string, Record<string, unknown>]
    expect(command).toBe('set_clip_controls')
    expect(args.kind).toBe('cc')
    expect(args.cc).toBe(1)
    const points = args.points as { tick: number; value: number }[]
    // One point per change: the default, up to 0.5, back down after the
    // two slots that hold it, up to 1, and back down again. Slot 101
    // repeats slot 100, so it is not a point of its own.
    expect(points.map(p => p.value)).toEqual([0, 0.5, 0, 1, 0])
    for (let i = 1; i < points.length; i++) {
      expect(points[i].tick).toBeGreaterThan(points[i - 1].tick)
    }
  })

  it('turns a centred bend lane back into the clip range', async () => {
    useMidiCcStore.getState().setValueAt('c1', BEND, 10, 1)
    invokeMock.mockReset()
    invokeMock.mockResolvedValue(null)
    await useMidiCcStore.getState().flushLane('t1', 'c1', BEND, LENGTH)
    const args = invokeMock.mock.calls[0][1] as Record<string, unknown>
    const points = args.points as { tick: number; value: number }[]
    expect(args.kind).toBe('pitchBend')
    expect(points[0].value).toBe(0) // centre stays centre
    expect(points.some(p => p.value === 1)).toBe(true) // full bend up
  })

  it('writes nothing for a lane left at its default', async () => {
    useMidiCcStore.getState().setValueAt('c1', MOD, 5, 0)
    invokeMock.mockReset()
    invokeMock.mockResolvedValue(null)
    await useMidiCcStore.getState().flushLane('t1', 'c1', MOD, LENGTH)
    const args = invokeMock.mock.calls[0][1] as Record<string, unknown>
    expect(args.points).toEqual([])
  })

  it('knows the lane a stored id refers to', () => {
    expect(laneDefinition('cc1')?.cc).toBe(1)
    expect(laneDefinition('pb')?.kind).toBe('pitchBend')
    expect(laneDefinition('cc74')?.label).toBe('CC74')
    expect(laneDefinition('nonsense')).toBeNull()
  })
})
