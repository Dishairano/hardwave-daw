import { create } from 'zustand'

export const STEPS_PER_PATTERN = 16

export const PATTERN_COLORS = [
  '#DC2626', '#EA580C', '#CA8A04', '#65A30D',
  '#059669', '#0891B2', '#2563EB', '#7C3AED',
  '#C026D3', '#DB2777', '#94A3B8', '#525252',
] as const

export interface Pattern {
  id: string
  name: string
  /** channelId -> array of step velocities (0..1; 0 = off). */
  steps: Record<string, number[]>
  color?: string
  /** Pattern length in steps; when omitted, derives from longest channel or STEPS_PER_PATTERN. */
  length?: number
}

interface PatternState {
  patterns: Pattern[]
  activeId: string

  setActive: (id: string) => void
  nextPattern: () => void
  prevPattern: () => void
  addPattern: () => void
  clonePattern: () => void
  deletePattern: () => void
  renamePattern: (id: string, name: string) => void

  setStep: (channelId: string, stepIndex: number, velocity: number) => void
  clearChannel: (channelId: string) => void
  setPatternColor: (id: string, color: string) => void
  setPatternLength: (id: string, length: number | undefined) => void
  getEffectiveLength: (id: string) => number

  serialize: () => string
  hydrate: (json: string | null) => void
}

function emptyPattern(n: number): Pattern {
  return {
    id: `pat-${Date.now()}-${Math.floor(Math.random() * 1000)}`,
    name: `Pattern ${n}`,
    steps: {},
    color: PATTERN_COLORS[(n - 1) % PATTERN_COLORS.length],
  }
}

const first = emptyPattern(1)

export const usePatternStore = create<PatternState>((set, get) => ({
  patterns: [first],
  activeId: first.id,

  setActive: (id) => {
    if (get().patterns.some(p => p.id === id)) set({ activeId: id })
  },

  nextPattern: () => {
    const { patterns, activeId } = get()
    const i = patterns.findIndex(p => p.id === activeId)
    if (i < 0) return
    const ni = (i + 1) % patterns.length
    set({ activeId: patterns[ni].id })
  },

  prevPattern: () => {
    const { patterns, activeId } = get()
    const i = patterns.findIndex(p => p.id === activeId)
    if (i < 0) return
    const ni = (i - 1 + patterns.length) % patterns.length
    set({ activeId: patterns[ni].id })
  },

  addPattern: () => {
    const next = emptyPattern(get().patterns.length + 1)
    set(s => ({ patterns: [...s.patterns, next], activeId: next.id }))
  },

  clonePattern: () => {
    const { patterns, activeId } = get()
    const src = patterns.find(p => p.id === activeId)
    if (!src) return
    const copy: Pattern = {
      id: `pat-${Date.now()}-${Math.floor(Math.random() * 1000)}`,
      name: `${src.name} (copy)`,
      steps: Object.fromEntries(Object.entries(src.steps).map(([k, v]) => [k, [...v]])),
      color: src.color,
      length: src.length,
    }
    set(s => ({ patterns: [...s.patterns, copy], activeId: copy.id }))
  },

  deletePattern: () => {
    const { patterns, activeId } = get()
    if (patterns.length <= 1) return
    const i = patterns.findIndex(p => p.id === activeId)
    const next = patterns.filter(p => p.id !== activeId)
    const newActive = next[Math.max(0, Math.min(i, next.length - 1))].id
    set({ patterns: next, activeId: newActive })
  },

  renamePattern: (id, name) => set(s => ({
    patterns: s.patterns.map(p => p.id === id ? { ...p, name } : p),
  })),

  setStep: (channelId, stepIndex, velocity) => set(s => {
    const idx = s.patterns.findIndex(p => p.id === s.activeId)
    if (idx < 0) return {}
    const patterns = s.patterns.slice()
    const pat = { ...patterns[idx], steps: { ...patterns[idx].steps } }
    const cur = pat.steps[channelId] ? [...pat.steps[channelId]] : new Array(STEPS_PER_PATTERN).fill(0)
    cur[stepIndex] = Math.max(0, Math.min(1, velocity))
    pat.steps[channelId] = cur
    patterns[idx] = pat
    return { patterns }
  }),

  setPatternColor: (id, color) => set(s => ({
    patterns: s.patterns.map(p => p.id === id ? { ...p, color } : p),
  })),

  setPatternLength: (id, length) => set(s => ({
    patterns: s.patterns.map(p => p.id === id ? { ...p, length } : p),
  })),

  getEffectiveLength: (id) => {
    const p = get().patterns.find(x => x.id === id)
    if (!p) return STEPS_PER_PATTERN
    if (p.length && p.length > 0) return p.length
    let max = 0
    for (const arr of Object.values(p.steps)) {
      let last = -1
      for (let i = arr.length - 1; i >= 0; i--) {
        if (arr[i] > 0) { last = i; break }
      }
      if (last + 1 > max) max = last + 1
    }
    return max > 0 ? Math.min(max, STEPS_PER_PATTERN) : STEPS_PER_PATTERN
  },

  clearChannel: (channelId) => set(s => {
    const idx = s.patterns.findIndex(p => p.id === s.activeId)
    if (idx < 0) return {}
    const patterns = s.patterns.slice()
    const pat = { ...patterns[idx], steps: { ...patterns[idx].steps } }
    pat.steps[channelId] = new Array(STEPS_PER_PATTERN).fill(0)
    patterns[idx] = pat
    return { patterns }
  }),

  serialize: () => {
    const { patterns, activeId } = get()
    return JSON.stringify({ v: 1, patterns, activeId })
  },

  hydrate: (json) => {
    if (!json) {
      const fresh = emptyPattern(1)
      set({ patterns: [fresh], activeId: fresh.id })
      return
    }
    try {
      const parsed = JSON.parse(json)
      if (parsed && Array.isArray(parsed.patterns) && parsed.patterns.length > 0) {
        const activeId = typeof parsed.activeId === 'string' && parsed.patterns.some((p: Pattern) => p.id === parsed.activeId)
          ? parsed.activeId
          : parsed.patterns[0].id
        const migrated: Pattern[] = parsed.patterns.map((p: Pattern, i: number) => ({
          ...p,
          color: p.color || PATTERN_COLORS[i % PATTERN_COLORS.length],
        }))
        set({ patterns: migrated, activeId })
        return
      }
    } catch {}
    const fresh = emptyPattern(1)
    set({ patterns: [fresh], activeId: fresh.id })
  },
}))
