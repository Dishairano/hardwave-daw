import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { invokeOrToast } from '../api/invoke'
import { useProjectStore } from './projectStore'
import { useHistoryStore } from './historyStore'

/**
 * The playlist's rows: FL's playlist tracks, kept with the song.
 *
 * A row has a name (empty reads "Track N"), a colour and two switches. Its
 * mute is FL's track switch: the clips on it go quiet, the channels they
 * play through keep running. The rows live in the project, so every change
 * here is an undo step and travels to the other person in a room.
 */
export interface PlaylistRow {
  name: string
  color: string | null
  muted: boolean
  soloed: boolean
}

interface PlaylistRowsState {
  rows: PlaylistRow[]
  /** The row last clicked, in the grid or on its name: F2 renames it. */
  selectedRow: number | null
  selectRow: (row: number | null) => void
  /** Read the rows again: after anything that may have changed them (an
   *  undo, an opened song, a room, an arrangement switch). */
  fetch: () => Promise<void>
  rename: (row: number, name: string) => Promise<void>
  setColor: (row: number, color: string | null) => Promise<void>
  setMuted: (row: number, muted: boolean) => Promise<void>
  setSoloed: (row: number, soloed: boolean) => Promise<void>
  /** Put `count` empty rows in above `at`. */
  insertRows: (at: number, count?: number) => Promise<void>
  /** Take rows out, with the clips on them. */
  deleteRows: (at: number, count?: number) => Promise<void>
}

/** A row as the store keeps it, for a row past the end of the list. */
export const EMPTY_ROW: PlaylistRow = { name: '', color: null, muted: false, soloed: false }

/** The rows with `row` changed, the list grown to reach it. */
function withRow(rows: PlaylistRow[], row: number, change: Partial<PlaylistRow>): PlaylistRow[] {
  const next = rows.slice()
  while (next.length <= row) next.push(EMPTY_ROW)
  next[row] = { ...next[row], ...change }
  return next
}

async function edit(cmd: string, args: Record<string, unknown>, label: string) {
  const since = Date.now()
  await invokeOrToast(cmd, args, { message: `${label} failed`, retry: false })
  useProjectStore.getState().markDirty()
  useHistoryStore.getState().push(label, since)
}

/** The clips on rows that came or went: the playlist reads them again. */
async function refreshClips() {
  const { useTrackStore } = await import('./trackStore')
  await useTrackStore.getState().fetchTracks()
}

export const usePlaylistRowsStore = create<PlaylistRowsState>((set, get) => ({
  rows: [],
  selectedRow: null,
  selectRow: (row) => set({ selectedRow: row }),

  fetch: async () => {
    try {
      const rows = await invoke<PlaylistRow[]>('get_playlist_rows')
      set({ rows: Array.isArray(rows) ? rows.map((r) => ({ ...EMPTY_ROW, ...r })) : [] })
    } catch {
      // A build without rows: the playlist shows unnamed ones.
    }
  },

  rename: async (row, name) => {
    const clean = name.trim()
    set({ rows: withRow(get().rows, row, { name: clean }) })
    try {
      await edit('set_row_name', { row, name: clean }, clean ? `Rename row ${row + 1} to "${clean}"` : `Clear the name of row ${row + 1}`)
    } finally {
      await get().fetch()
    }
  },

  setColor: async (row, color) => {
    set({ rows: withRow(get().rows, row, { color }) })
    try {
      await edit('set_row_color', { row, color }, `Colour row ${row + 1}`)
    } finally {
      await get().fetch()
    }
  },

  setMuted: async (row, muted) => {
    set({ rows: withRow(get().rows, row, { muted }) })
    try {
      await edit('set_row_mute', { row, muted }, muted ? `Mute row ${row + 1}` : `Unmute row ${row + 1}`)
    } finally {
      await get().fetch()
    }
  },

  setSoloed: async (row, soloed) => {
    set({ rows: withRow(get().rows, row, { soloed }) })
    try {
      await edit('set_row_solo', { row, soloed }, soloed ? `Solo row ${row + 1}` : `Unsolo row ${row + 1}`)
    } finally {
      await get().fetch()
    }
  },

  insertRows: async (at, count = 1) => {
    try {
      await edit('insert_rows', { at, count }, count === 1 ? `Insert a row above row ${at + 1}` : `Insert ${count} rows`)
    } finally {
      await Promise.all([get().fetch(), refreshClips()])
    }
  },

  deleteRows: async (at, count = 1) => {
    try {
      await edit('delete_rows', { at, count }, count === 1 ? `Delete row ${at + 1}` : `Delete ${count} rows`)
    } finally {
      await Promise.all([get().fetch(), refreshClips()])
    }
  },
}))
