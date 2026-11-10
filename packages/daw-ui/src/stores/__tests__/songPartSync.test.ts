import { describe, it, expect, vi } from 'vitest'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }))
const handlers = new Map<string, (ev: { payload: unknown }) => void>()
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, cb: (ev: { payload: unknown }) => void) => {
    handlers.set(name, cb)
    return Promise.resolve(() => {})
  },
}))
// The project store keeps its recent songs in browser storage.
vi.stubGlobal('localStorage', {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
})

const { startSongPartSync } = await import('../songPartSync')
const { useProjectStore } = await import('../projectStore')
const { useTrackStore } = await import('../trackStore')

describe('taking the song from a room', () => {
  it('shows their tracks and keeps the song open here from being written over', async () => {
    useProjectStore.setState({ filePath: 'C:/Songs/Mine.hwp', projectName: 'Mine', dirty: false })
    invokeMock.mockResolvedValue([
      { id: 'k', name: 'HostKick', kind: 'Audio', volume_db: 0, pan: 0, muted: false, soloed: false, clips: [] },
    ])
    startSongPartSync()
    await vi.waitFor(() => expect(handlers.has('daw:songTaken')).toBe(true))

    handlers.get('daw:songTaken')!({ payload: { name: 'Room Song' } })

    await vi.waitFor(() => expect(useTrackStore.getState().tracks.map(t => t.name)).toEqual(['HostKick']))
    expect(invokeMock).toHaveBeenCalledWith('get_tracks_with_clips')
    expect(useProjectStore.getState()).toMatchObject({ filePath: null, projectName: 'Room Song', dirty: true })
  })

  it('reads the rows again when the other person changed one', async () => {
    const { usePlaylistRowsStore } = await import('../playlistRowsStore')
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === 'get_playlist_rows' ? [{ name: 'Drums', color: null, muted: true, soloed: false }] : []))
    startSongPartSync()
    await vi.waitFor(() => expect(handlers.has('daw:songChanged')).toBe(true))
    handlers.get('daw:songChanged')!({ payload: 'rows' })
    await vi.waitFor(() => expect(usePlaylistRowsStore.getState().rows[0]?.name).toBe('Drums'))
    expect(usePlaylistRowsStore.getState().rows[0].muted).toBe(true)
  })
})
