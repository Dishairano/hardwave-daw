import { describe, expect, it, vi } from 'vitest'

const calls: string[] = []
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) => {
    calls.push(cmd)
    if (cmd === 'get_track_with_clips') return { id: 'kick', kind: 'Midi', name: 'Big Kick', clips: [] }
    if (cmd === 'get_playlist_rows') return [{ name: 'Big Kick', color: null, muted: false, soloed: false }]
    return null
  }),
}))

const { useTrackStore } = await import('./trackStore')
const { usePlaylistRowsStore } = await import('./playlistRowsStore')

describe('renaming a track', () => {
  it('reads the playlist rows again, as the row made for it takes the new name', async () => {
    useTrackStore.setState({ tracks: [{ id: 'kick', kind: 'Midi', name: 'Kick', clips: [] }] as never })
    await useTrackStore.getState().renameTrack('kick', 'Big Kick')
    expect(calls).toContain('set_track_name')
    await vi.waitFor(() => expect(usePlaylistRowsStore.getState().rows[0]?.name).toBe('Big Kick'))
  })
})
