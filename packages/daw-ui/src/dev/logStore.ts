import { create } from 'zustand'

export type LogLevel = 'info' | 'pass' | 'fail' | 'event'

export interface LogEntry {
  ts: number
  level: LogLevel
  test?: string
  message: string
  expected?: unknown
  actual?: unknown
}

interface LogStore {
  entries: LogEntry[]
  append: (entry: Omit<LogEntry, 'ts'>) => void
  clear: () => void
  exportText: () => string
}

const MAX_ENTRIES = 2000

function fmt(val: unknown): string {
  if (val === undefined) return ''
  if (typeof val === 'object') return JSON.stringify(val)
  return String(val)
}

export const useLogStore = create<LogStore>((set, get) => ({
  entries: [],
  append: (e) => set((s) => {
    const next = [...s.entries, { ...e, ts: Date.now() }]
    if (next.length > MAX_ENTRIES) next.splice(0, next.length - MAX_ENTRIES)
    return { entries: next }
  }),
  clear: () => set({ entries: [] }),
  exportText: () => {
    const lines = get().entries.map((e) => {
      const iso = new Date(e.ts).toISOString()
      const tag = e.level.toUpperCase().padEnd(5)
      const test = e.test ? `[${e.test}] ` : ''
      let body = `${iso} ${tag} ${test}${e.message}`
      if (e.expected !== undefined || e.actual !== undefined) {
        body += ` | expected=${fmt(e.expected)} actual=${fmt(e.actual)}`
      }
      return body
    })
    return lines.join('\n')
  },
}))
