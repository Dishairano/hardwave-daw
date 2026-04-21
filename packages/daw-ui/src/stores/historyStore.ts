import { create } from 'zustand'

export interface HistoryEntry {
  id: string
  label: string
  time: number
}

interface HistoryState {
  entries: HistoryEntry[]
  cursor: number
  push: (label: string) => void
  undoOne: () => void
  redoOne: () => void
  jumpTo: (target: number, undo: () => Promise<boolean>, redo: () => Promise<boolean>) => Promise<void>
  clear: () => void
}

const MAX_ENTRIES = 200

let nextId = 1

export const useHistoryStore = create<HistoryState>((set, get) => ({
  entries: [],
  cursor: 0,

  push: (label) => set(s => {
    const kept = s.entries.slice(0, s.cursor)
    const entry: HistoryEntry = {
      id: `h${nextId++}`,
      label,
      time: Date.now(),
    }
    const next = [...kept, entry]
    const overflow = Math.max(0, next.length - MAX_ENTRIES)
    const trimmed = overflow > 0 ? next.slice(overflow) : next
    return { entries: trimmed, cursor: trimmed.length }
  }),

  undoOne: () => set(s => ({ cursor: Math.max(0, s.cursor - 1) })),
  redoOne: () => set(s => ({ cursor: Math.min(s.entries.length, s.cursor + 1) })),

  jumpTo: async (target, undo, redo) => {
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
  },

  clear: () => set({ entries: [], cursor: 0 }),
}))
