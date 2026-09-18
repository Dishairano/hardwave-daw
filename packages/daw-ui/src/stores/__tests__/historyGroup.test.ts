import { describe, it, expect, vi, beforeEach } from 'vitest'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useTrackStore } = await import('../trackStore')
const { useHistoryStore } = await import('../historyStore')

describe('history groups', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockResolvedValue([])
    useHistoryStore.getState().clear()
    useTrackStore.setState({ tracks: [], tracksById: {} } as never)
  })

  /// The engine keeps one snapshot for a whole group, so the undo list must
  /// show one entry. Every command inside used to push its own label.
  it('a grouped gesture is one entry in the undo list', async () => {
    const store = useTrackStore.getState()
    await store.beginHistoryGroup()
    await store.setClipMuted('t', 'c1', true)
    await store.setClipMuted('t', 'c2', true)
    await store.endHistoryGroup('Record 3 loop passes')

    const labels = useHistoryStore.getState().entries.map(e => e.label)
    expect(labels).toEqual(['Record 3 loop passes'])
  })

  it('commands outside a group keep their own entries', async () => {
    const store = useTrackStore.getState()
    await store.setClipMuted('t', 'c1', true)
    await store.setClipMuted('t', 'c2', false)
    expect(useHistoryStore.getState().entries).toHaveLength(2)
  })

  it('a group that ends without a label leaves no entry', async () => {
    const store = useTrackStore.getState()
    await store.beginHistoryGroup()
    await store.setClipMuted('t', 'c1', true)
    await store.endHistoryGroup(null)
    expect(useHistoryStore.getState().entries).toHaveLength(0)
  })

  it('labels come back once the group has closed', async () => {
    const store = useTrackStore.getState()
    await store.beginHistoryGroup()
    await store.endHistoryGroup('Paint 4 clips')
    await store.setClipMuted('t', 'c1', true)
    const labels = useHistoryStore.getState().entries.map(e => e.label)
    expect(labels).toEqual(['Paint 4 clips', 'Mute clip'])
  })
})
