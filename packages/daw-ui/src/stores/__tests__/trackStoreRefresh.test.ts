import { describe, it, expect, vi, beforeEach } from 'vitest'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useTrackStore } = await import('../trackStore')

function track(id: string, volumeDb: number) {
  return {
    id,
    name: id,
    kind: 'Audio',
    volume_db: volumeDb,
    pan: 0,
    muted: false,
    soloed: false,
    clips: [],
  }
}

describe('refreshTrack', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    useTrackStore.setState({
      tracks: [track('a', 0), track('b', -6)] as never,
      tracksById: { a: track('a', 0), b: track('b', -6) } as never,
    })
  })

  it('asks for one track, not the whole project', async () => {
    invokeMock.mockResolvedValue({ ...track('b', -3) })
    await useTrackStore.getState().refreshTrack('b')

    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('get_track_with_clips', { trackId: 'b' })
  })

  it('updates that track and leaves the others alone', async () => {
    invokeMock.mockResolvedValue({ ...track('b', -3) })
    await useTrackStore.getState().refreshTrack('b')

    const { tracks, tracksById } = useTrackStore.getState()
    expect(tracks.find(t => t.id === 'b')?.volume_db).toBe(-3)
    expect(tracks.find(t => t.id === 'a')?.volume_db).toBe(0)
    expect(tracksById.b.volume_db).toBe(-3)
    expect(tracks).toHaveLength(2)
  })

  it('gives a track with no clips an empty clip list rather than undefined', async () => {
    const withoutClips = { ...track('b', -3) } as Record<string, unknown>
    delete withoutClips.clips
    invokeMock.mockResolvedValue(withoutClips)
    await useTrackStore.getState().refreshTrack('b')

    expect(useTrackStore.getState().tracks.find(t => t.id === 'b')?.clips).toEqual([])
  })

  it('falls back to a full refresh when the track is gone', async () => {
    // First call answers "no such track", so the store reloads everything.
    invokeMock.mockResolvedValueOnce(null).mockResolvedValueOnce([track('a', 0)])
    await useTrackStore.getState().refreshTrack('b')

    expect(invokeMock).toHaveBeenNthCalledWith(2, 'get_tracks_with_clips')
    expect(useTrackStore.getState().tracks).toHaveLength(1)
  })

  it('falls back to a full refresh against a backend without the endpoint', async () => {
    invokeMock
      .mockRejectedValueOnce(new Error('command not found'))
      .mockResolvedValueOnce([track('a', 0), track('b', -6)])
    await useTrackStore.getState().refreshTrack('b')

    expect(invokeMock).toHaveBeenNthCalledWith(2, 'get_tracks_with_clips')
    expect(useTrackStore.getState().tracks).toHaveLength(2)
  })
})
