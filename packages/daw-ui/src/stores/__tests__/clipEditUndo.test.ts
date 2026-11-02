import { describe, it, expect, vi, beforeEach } from 'vitest'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useTrackStore } = await import('../trackStore')

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
})
