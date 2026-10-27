import { describe, it, expect, vi, beforeEach } from 'vitest'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useTrackStore } = await import('../trackStore')

/** The labels sent to the engine for its undo steps, in order. */
function labelsSent(): string[] {
  return invokeMock.mock.calls
    .filter(([cmd]) => cmd === 'label_undo_step')
    .map(([, args]) => (args as { label: string }).label)
}

describe('history groups', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockResolvedValue([])
    useTrackStore.setState({ tracks: [], tracksById: {} } as never)
  })

  /// The engine keeps one snapshot for a whole group, so the group gets one
  /// label. Every command inside used to push its own.
  it('a grouped gesture is named once', async () => {
    const store = useTrackStore.getState()
    await store.beginHistoryGroup()
    await store.setClipMuted('t', 'c1', true)
    await store.setClipMuted('t', 'c2', true)
    await store.endHistoryGroup('Record 3 loop passes')
    expect(labelsSent()).toEqual(['Record 3 loop passes'])
  })

  it('commands outside a group name their own steps', async () => {
    const store = useTrackStore.getState()
    await store.setClipMuted('t', 'c1', true)
    await store.setClipMuted('t', 'c2', false)
    expect(labelsSent()).toHaveLength(2)
  })

  it('a group that ends without a label names nothing', async () => {
    const store = useTrackStore.getState()
    await store.beginHistoryGroup()
    await store.setClipMuted('t', 'c1', true)
    await store.endHistoryGroup(null)
    expect(labelsSent()).toHaveLength(0)
  })

  it('labels come back once the group has closed', async () => {
    const store = useTrackStore.getState()
    await store.beginHistoryGroup()
    await store.endHistoryGroup('Paint 4 clips')
    await store.setClipMuted('t', 'c1', true)
    expect(labelsSent()).toEqual(['Paint 4 clips', 'Mute clip'])
  })
})
