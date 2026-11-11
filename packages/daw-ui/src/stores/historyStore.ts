import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

/** One row of the History panel: one of the engine's undo steps. */
export interface HistoryEntry {
  id: string
  /** The app's own name for the edit, or a template such as
   *  'Add {n} notes: "{name}"' that the engine read off the song. */
  label: string
  name?: string
  old?: string
  n?: number
  time: number
}

interface UndoHistory {
  entries: { label: string; name?: string; old?: string; n?: number; time: number }[]
  cursor: number
  startsAtOpen: boolean
}

interface HistoryState {
  entries: HistoryEntry[]
  /** How many entries are applied; the rest can be redone. */
  cursor: number
  /** Undoing every step gets back to the song as it was opened (not
   *  once old steps have gone past the engine's cap). */
  startsAtOpen: boolean
  /** Name the edit a command made. `since` is `Date.now()` from just
   *  before the command was sent: the engine names its newest step only
   *  if it was taken since then, so a command that took no step names
   *  nothing rather than the step before it. */
  push: (label: string, since: number) => void
  /** Read the list from the engine again. */
  refresh: () => Promise<void>
  /** Read the list again whenever the engine says its steps changed
   *  (`daw:historyChanged`), until the returned function is called. */
  follow: () => () => void
  undoOne: () => void
  redoOne: () => void
  jumpTo: (target: number, undo: () => Promise<boolean>, redo: () => Promise<boolean>) => Promise<void>
}

/**
 * The History panel's list is the engine's undo stack, read from it.
 *
 * It used to be a log of its own, pushed to by the commands that had a
 * label, beside the real undo steps. The two drifted apart: a note drawn
 * in the piano roll or a fader moved on a desk was an undo step the list
 * never showed, and a label pushed for a command that took no step was a
 * row that undid something else. Now a label only names a step the engine
 * took, and steps without one are described by the engine.
 */
/** The read of the list on its way, and whether it was asked for again
 *  meanwhile. */
let reading: Promise<void> | null = null
let readAgain = false

export const useHistoryStore = create<HistoryState>((set, get) => ({
  entries: [],
  cursor: 0,
  startsAtOpen: true,

  push: (label, since) => {
    invoke('label_undo_step', { label, since }).catch(() => {})
  },

  refresh: async () => {
    // One read at a time: asked again while one is on its way, the list
    // is read once more after it, never several times side by side.
    if (reading) {
      readAgain = true
      return reading
    }
    reading = (async () => {
      do {
        readAgain = false
        const h = await invoke<UndoHistory>('get_undo_history').catch(() => null)
        if (h) {
          set({
            entries: h.entries.map((e, i) => ({ ...e, id: `h${i}` })),
            cursor: h.cursor,
            startsAtOpen: h.startsAtOpen,
          })
        }
      } while (readAgain)
    })()
    try {
      await reading
    } finally {
      reading = null
    }
  },

  follow: () => {
    let stopped = false
    let unlisten: (() => void) | null = null
    import('@tauri-apps/api/event')
      .then(({ listen }) => listen('daw:historyChanged', () => { void get().refresh() }))
      .then((un) => { if (stopped) un(); else unlisten = un })
      .catch(() => { /* no events outside the app; the list was read when it opened */ })
    return () => {
      stopped = true
      if (unlisten) unlisten()
    }
  },

  undoOne: () => { void get().refresh() },
  redoOne: () => { void get().refresh() },

  jumpTo: async (target, undo, redo) => {
    await get().refresh()
    const { cursor, entries } = get()
    const clamped = Math.max(0, Math.min(entries.length, target))
    if (clamped === cursor) return
    if (clamped < cursor) {
      for (let i = 0; i < cursor - clamped; i++) {
        const ok = await undo()
        if (!ok) break
      }
    } else {
      for (let i = 0; i < clamped - cursor; i++) {
        const ok = await redo()
        if (!ok) break
      }
    }
    await get().refresh()
  },
}))
