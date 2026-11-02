import { describe, it, expect, vi, beforeEach } from 'vitest'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useTrackStore } = await import('../trackStore')
const { useProjectStore } = await import('../projectStore')

/** The commands sent, in order, without the reads that follow an edit. */
function sent(): string[] {
  const reads = new Set(['get_tracks_with_clips', 'get_tracks', 'get_track_with_clips', 'get_undo_history'])
  return invokeMock.mock.calls.map(([cmd]) => cmd as string).filter(cmd => !reads.has(cmd))
}

function labelsSent(): string[] {
  return invokeMock.mock.calls
    .filter(([cmd]) => cmd === 'label_undo_step')
    .map(([, args]) => (args as { label: string }).label)
}

function clip(id: string, position: number) {
  return { id, name: id, kind: 'audio', position_ticks: position, length_ticks: 960, fadeInTicks: 0, fadeOutTicks: 0 }
}

describe('clip edits and undo', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockResolvedValue([])
    const tracks = [
      { id: 'a', kind: 'Audio', clips: [clip('c1', 0), clip('c2', 960)] },
      { id: 'b', kind: 'Audio', clips: [clip('c3', 0)] },
    ]
    useTrackStore.setState({
      tracks,
      tracksById: Object.fromEntries(tracks.map(t => [t.id, t])),
      selectedClipId: 'c1',
      selectedClipIds: new Set(['c1', 'c2', 'c3']),
    } as never)
  })

  it('deleting several clips is one undo step', async () => {
    await useTrackStore.getState().deleteSelectedClips()
    expect(sent()).toEqual([
      'begin_history_group',
      'delete_clip', 'delete_clip', 'delete_clip',
      'end_history_group',
      'label_undo_step',
    ])
    expect(labelsSent()).toEqual(['Delete 3 clips'])
  })

  it('a click on a clip that moved nothing is no undo step', async () => {
    await useTrackStore.getState().commitClipDrag(
      [{ trackId: 'a', clipId: 'c1', fromPositionTicks: 0, positionTicks: 0 }],
      'Move clip',
    )
    expect(sent()).toEqual([])
  })

  it('a resize sends only the new length, as one step', async () => {
    await useTrackStore.getState().commitClipDrag(
      [{ trackId: 'a', clipId: 'c1', fromPositionTicks: 0, positionTicks: 0, fromLengthTicks: 960, lengthTicks: 1920 }],
      'Resize clip',
    )
    expect(sent()).toEqual(['begin_history_group', 'resize_clip', 'end_history_group', 'label_undo_step'])
    expect(labelsSent()).toEqual(['Resize clip'])
  })

  it('a group of clips moved together is one step', async () => {
    await useTrackStore.getState().commitClipDrag(
      [
        { trackId: 'a', clipId: 'c1', fromPositionTicks: 0, positionTicks: 480 },
        { trackId: 'a', clipId: 'c2', fromPositionTicks: 960, positionTicks: 1440 },
        { trackId: 'b', clipId: 'c3', fromPositionTicks: 0, positionTicks: 480 },
      ],
      'Move 3 clips',
    )
    expect(sent()).toEqual([
      'begin_history_group',
      'move_clip', 'move_clip', 'move_clip',
      'end_history_group',
      'label_undo_step',
    ])
    expect(labelsSent()).toEqual(['Move 3 clips'])
  })

  it('a drag that already opened its step keeps it open until the release closes it', async () => {
    // A move onto another track opens the step while the clip is still held.
    const store = useTrackStore.getState()
    await store.beginHistoryGroup()
    await store.moveClipToTrack('a', 'b', 'c1', 480)
    await store.commitClipDrag(
      [{ trackId: 'b', clipId: 'c1', fromPositionTicks: 480, positionTicks: 960 }],
      'Move clip to track',
    )
    expect(sent()).toEqual([
      'begin_history_group',
      'move_clip_to_track',
      'begin_history_group',
      'move_clip',
      'end_history_group',
      'label_undo_step',
    ])
    expect(labelsSent()).toEqual(['Move clip to track'])
  })

  it('a moved clip is a change to save, a click on one is not', async () => {
    useProjectStore.setState({ dirty: false })
    const store = useTrackStore.getState()
    await store.commitClipDrag([{ trackId: 'a', clipId: 'c1', fromPositionTicks: 0, positionTicks: 0 }], 'Move clip')
    expect(useProjectStore.getState().dirty).toBe(false)
    await store.commitClipDrag([{ trackId: 'a', clipId: 'c1', fromPositionTicks: 0, positionTicks: 960 }], 'Move clip')
    expect(useProjectStore.getState().dirty).toBe(true)
  })
})
