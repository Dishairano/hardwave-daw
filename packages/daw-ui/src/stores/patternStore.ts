import { create } from 'zustand'

export const STEPS_PER_PATTERN = 16

export interface Pattern {
  id: string
  name: string
  /** channelId -> array of step velocities (0..1; 0 = off). */
  steps: Record<string, number[]>
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
}

function emptyPattern(n: number): Pattern {
  return {
    id: `pat-${Date.now()}-${Math.floor(Math.random() * 1000)}`,
    name: `Pattern ${n}`,
    steps: {},
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

  clearChannel: (channelId) => set(s => {
    const idx = s.patterns.findIndex(p => p.id === s.activeId)
    if (idx < 0) return {}
    const patterns = s.patterns.slice()
    const pat = { ...patterns[idx], steps: { ...patterns[idx].steps } }
    pat.steps[channelId] = new Array(STEPS_PER_PATTERN).fill(0)
    patterns[idx] = pat
    return { patterns }
  }),
}))
