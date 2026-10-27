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

/** The same for the VCA store, which lives outside any one window. */
let started = false
export function startSongPartSync(): void {
  if (started) return
  started = true
  import('@tauri-apps/api/event')
    .then(({ listen }) => listen<string>('daw:songChanged', (ev) => {
      if (ev.payload === 'vcas') {
        void import('./vcaStore').then(({ useVcaStore }) => useVcaStore.getState().fetch())
      }
    }))
    .catch(() => { /* outside the app */ })
}
