import { useEffect } from 'react'

/**
 * A room partner changed a part of the song that a window shows on its own
 * (the arrangement list, the macros, the modulation, the VCAs): the engine
 * says which with `daw:songChanged`, and the window reads it again. They
 * kept what they had read, and the next edit wrote that stale copy back
 * over the partner's change.
 */
export function useSongPartChanged(section: string, reload: () => void): void {
  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    import('@tauri-apps/api/event')
      .then(({ listen }) => listen<string>('daw:songChanged', (ev) => {
        if (ev.payload === section) reload()
      }))
      .then((u) => { if (cancelled) u(); else unlisten = u })
      .catch(() => { /* outside the app */ })
    return () => { cancelled = true; unlisten?.() }
  }, [section, reload])
}

/**
 * The whole song taken from the other side of a room ("Get the song from
 * them"): the tracks are read again and the song is theirs, unsaved, so
 * saving asks where instead of writing over the song that was open here.
 * Its rack, markers, tempo and other parts arrive by their own events.
 */
export async function takeRoomSong(name: string): Promise<void> {
  const [{ useProjectStore }, { useTrackStore }] = await Promise.all([
    import('./projectStore'),
    import('./trackStore'),
  ])
  useProjectStore.setState({ filePath: null, projectName: name || 'Untitled', dirty: true })
  await useTrackStore.getState().fetchTracks()
}

/** The same for the VCA store and the playlist's rows, which live outside
 *  any one window. */
let started = false
export function startSongPartSync(): void {
  if (started) return
  started = true
  import('@tauri-apps/api/event')
    .then(({ listen }) => Promise.all([
      listen<string>('daw:songChanged', (ev) => {
        if (ev.payload === 'vcas') {
          void import('./vcaStore').then(({ useVcaStore }) => useVcaStore.getState().fetch())
        }
        // A row renamed, coloured or switched by the other person.
        if (ev.payload === 'rows') {
          void import('./playlistRowsStore').then(({ usePlaylistRowsStore }) => usePlaylistRowsStore.getState().fetch())
        }
      }),
      listen<{ name: string }>('daw:songTaken', (ev) => {
        void takeRoomSong(ev.payload?.name ?? '')
      }),
    ]))
    .catch(() => { /* outside the app */ })
}
