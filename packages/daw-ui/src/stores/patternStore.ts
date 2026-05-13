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
  /** channelId -> per-step pan (-1..1). Defaults to 0 when absent. */
  panSteps?: Record<string, number[]>
  /** channelId -> per-step pitch offset in semitones (-12..12). Defaults to 0. */
  pitchSteps?: Record<string, number[]>
  /** channelId -> per-step Mod-X / filter cutoff (0..1 normalized). Defaults to 1. Persisted as `filterSteps` for backward-compat with v0.169 saves. */
  filterSteps?: Record<string, number[]>
  /** channelId -> per-step Shift / gate length (0..1 normalized). Defaults to 1. Persisted as `gateSteps` for backward-compat. */
  gateSteps?: Record<string, number[]>
  /** Ship 4 — Note pitch offset per step in semitones (-12..12). FL "Note" graph editor mode — useful for tuning drum hits or chromatic FX. Defaults to 0. */
  noteSteps?: Record<string, number[]>
  /** Ship 4 — Release velocity per step (0..1). FL "Release" graph mode. Defaults to 1. */
  releaseSteps?: Record<string, number[]>
  /** Ship 4 — Mod-Y per step (0..1). FL "Mod-Y" graph mode — usually mapped to filter resonance. Defaults to 0.5. */
  modYSteps?: Record<string, number[]>
  /** Ship 4 — Repeat count per step (1..8 retriggers). FL "Rep" graph mode for rapid retriggers. Defaults to 1. */
  repSteps?: Record<string, number[]>
  color?: string
  /** Pattern length in steps; when omitted, derives from longest channel or STEPS_PER_PATTERN. */
  length?: number
}

export type StepGraphKind = 'note' | 'velocity' | 'release' | 'pitch' | 'modX' | 'modY' | 'pan' | 'shift' | 'rep'

/** Defaults per non-velocity graph dimension. */
export const STEP_GRAPH_DEFAULTS: Record<Exclude<StepGraphKind, 'velocity'>, number> = {
  note: 0,
  release: 1,
  pitch: 0,
  modX: 1,
  modY: 0.5,
  pan: 0,
  shift: 1,
  rep: 1,
}

/** [min, max] per non-velocity dimension. */
export const STEP_GRAPH_RANGES: Record<Exclude<StepGraphKind, 'velocity'>, [number, number]> = {
  note: [-12, 12],
  release: [0, 1],
  pitch: [-12, 12],
  modX: [0, 1],
  modY: [0, 1],
  pan: [-1, 1],
  shift: [0, 1],
  rep: [1, 8],
}

/** User-facing labels for each graph mode. */
export const STEP_GRAPH_LABELS: Record<StepGraphKind, string> = {
  note: 'Note',
  velocity: 'Velocity',
  release: 'Release',
  pitch: 'Fine pitch',
  modX: 'Mod-X',
  modY: 'Mod-Y',
  pan: 'Pan',
  shift: 'Shift',
  rep: 'Rep',
}

interface PatternState {
  patterns: Pattern[]
  activeId: string

  setActive: (id: string) => void
  nextPattern: () => void
  prevPattern: () => void
  addPattern: () => void
  /** Insert a fresh empty pattern immediately after the active one
   * (FL Studio Patterns menu → Insert one, Shift+Ctrl+Ins). Returns
   * the new pattern id. */
  insertAfterActive: () => string
  clonePattern: () => void
  deletePattern: () => void
  renamePattern: (id: string, name: string) => void
  /** Reorder the active pattern up / down in the list. No-op when
   * already at the top / bottom edge. */
  moveActiveUp: () => void
  moveActiveDown: () => void
  /** Set the active pattern's color to a random entry from
   * PATTERN_COLORS — FL Patterns menu → Random color. */
  randomColorActive: () => void
  /** Activate the lowest-index pattern with no notes anywhere, or
   * append a new one when every pattern has content. FL Patterns
   * menu → Find first empty (Shift+F4). */
  findFirstEmpty: () => void

  setStep: (channelId: string, stepIndex: number, velocity: number) => void
  setPanStep: (channelId: string, stepIndex: number, pan: number) => void
  setPitchStep: (channelId: string, stepIndex: number, semitones: number) => void
  /** Legacy alias for Mod-X — same underlying `filterSteps` field. */
  setFilterStep: (channelId: string, stepIndex: number, cutoff: number) => void
  /** Legacy alias for Shift — same underlying `gateSteps` field. */
  setGateStep: (channelId: string, stepIndex: number, gate: number) => void
  /** Ship 4 — 4 new FL graph modes. Each writes the matching pat[*Steps] map. */
  setNoteStep: (channelId: string, stepIndex: number, semitones: number) => void
  setReleaseStep: (channelId: string, stepIndex: number, release: number) => void
  setModYStep: (channelId: string, stepIndex: number, resonance: number) => void
  setRepStep: (channelId: string, stepIndex: number, repeats: number) => void
  getStepGraphValues: (channelId: string, kind: StepGraphKind) => number[]
  clearChannel: (channelId: string) => void
  setPatternColor: (id: string, color: string) => void
  setPatternLength: (id: string, length: number | undefined) => void
  getEffectiveLength: (id: string) => number

  serialize: () => string
  hydrate: (json: string | null) => void
}

type GraphKey =
  | 'panSteps' | 'pitchSteps' | 'filterSteps' | 'gateSteps'
  | 'noteSteps' | 'releaseSteps' | 'modYSteps' | 'repSteps'

function updateStepGraph(
  s: PatternState,
  channelId: string,
  stepIndex: number,
  key: GraphKey,
  value: number,
  defaultValue: number,
) {
  const idx = s.patterns.findIndex(p => p.id === s.activeId)
  if (idx < 0) return {}
  const patterns = s.patterns.slice()
  const pat = { ...patterns[idx] }
  const graph = { ...(pat[key] ?? {}) }
  const cur = graph[channelId] ? [...graph[channelId]] : new Array(STEPS_PER_PATTERN).fill(defaultValue)
  cur[stepIndex] = value
  graph[channelId] = cur
  pat[key] = graph
  patterns[idx] = pat
  return { patterns }
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

  insertAfterActive: () => {
    const { patterns, activeId } = get()
    const next = emptyPattern(patterns.length + 1)
    const idx = patterns.findIndex(p => p.id === activeId)
    const insertAt = idx < 0 ? patterns.length : idx + 1
    const before = patterns.slice(0, insertAt)
    const after = patterns.slice(insertAt)
    set({ patterns: [...before, next, ...after], activeId: next.id })
    return next.id
  },

  moveActiveUp: () => {
    const { patterns, activeId } = get()
    const i = patterns.findIndex(p => p.id === activeId)
    if (i <= 0) return
    const next = patterns.slice()
    ;[next[i - 1], next[i]] = [next[i], next[i - 1]]
    set({ patterns: next })
  },

  moveActiveDown: () => {
    const { patterns, activeId } = get()
    const i = patterns.findIndex(p => p.id === activeId)
    if (i < 0 || i >= patterns.length - 1) return
    const next = patterns.slice()
    ;[next[i + 1], next[i]] = [next[i], next[i + 1]]
    set({ patterns: next })
  },

  randomColorActive: () => {
    const color = PATTERN_COLORS[Math.floor(Math.random() * PATTERN_COLORS.length)]
    const { activeId } = get()
    set(s => ({ patterns: s.patterns.map(p => p.id === activeId ? { ...p, color } : p) }))
  },

  findFirstEmpty: () => {
    const { patterns } = get()
    const isEmpty = (p: Pattern) =>
      Object.values(p.steps).every(arr => arr.every(v => v === 0)) &&
      (!p.panSteps || Object.values(p.panSteps).every(arr => arr.every(v => v === 0))) &&
      (!p.pitchSteps || Object.values(p.pitchSteps).every(arr => arr.every(v => v === 0)))
    const found = patterns.find(isEmpty)
    if (found) {
      set({ activeId: found.id })
      return
    }
    // No empty pattern — append a fresh one and activate it.
    const next = emptyPattern(patterns.length + 1)
    set({ patterns: [...patterns, next], activeId: next.id })
  },

  clonePattern: () => {
    const { patterns, activeId } = get()
    const src = patterns.find(p => p.id === activeId)
    if (!src) return
    const cloneMap = (m?: Record<string, number[]>) =>
      m ? Object.fromEntries(Object.entries(m).map(([k, v]) => [k, [...v]])) : undefined
    const copy: Pattern = {
      id: `pat-${Date.now()}-${Math.floor(Math.random() * 1000)}`,
      name: `${src.name} (copy)`,
      steps: Object.fromEntries(Object.entries(src.steps).map(([k, v]) => [k, [...v]])),
      panSteps: cloneMap(src.panSteps),
      pitchSteps: cloneMap(src.pitchSteps),
      filterSteps: cloneMap(src.filterSteps),
      gateSteps: cloneMap(src.gateSteps),
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

  setPanStep: (channelId, stepIndex, pan) => set(s => updateStepGraph(s, channelId, stepIndex, 'panSteps', Math.max(-1, Math.min(1, pan)), STEP_GRAPH_DEFAULTS.pan)),
  setPitchStep: (channelId, stepIndex, semitones) => set(s => updateStepGraph(s, channelId, stepIndex, 'pitchSteps', Math.max(-12, Math.min(12, semitones)), STEP_GRAPH_DEFAULTS.pitch)),
  setFilterStep: (channelId, stepIndex, cutoff) => set(s => updateStepGraph(s, channelId, stepIndex, 'filterSteps', Math.max(0, Math.min(1, cutoff)), STEP_GRAPH_DEFAULTS.modX)),
  setGateStep: (channelId, stepIndex, gate) => set(s => updateStepGraph(s, channelId, stepIndex, 'gateSteps', Math.max(0, Math.min(1, gate)), STEP_GRAPH_DEFAULTS.shift)),
  setNoteStep:    (channelId, stepIndex, semitones) => set(s => updateStepGraph(s, channelId, stepIndex, 'noteSteps',    Math.max(-12, Math.min(12, semitones)), STEP_GRAPH_DEFAULTS.note)),
  setReleaseStep: (channelId, stepIndex, release)   => set(s => updateStepGraph(s, channelId, stepIndex, 'releaseSteps', Math.max(0, Math.min(1, release)),     STEP_GRAPH_DEFAULTS.release)),
  setModYStep:    (channelId, stepIndex, resonance) => set(s => updateStepGraph(s, channelId, stepIndex, 'modYSteps',    Math.max(0, Math.min(1, resonance)),   STEP_GRAPH_DEFAULTS.modY)),
  setRepStep:     (channelId, stepIndex, repeats)   => set(s => updateStepGraph(s, channelId, stepIndex, 'repSteps',     Math.max(1, Math.min(8, Math.round(repeats))), STEP_GRAPH_DEFAULTS.rep)),

  getStepGraphValues: (channelId, kind) => {
    const pat = get().patterns.find(p => p.id === get().activeId)
    if (!pat) return new Array(STEPS_PER_PATTERN).fill(0)
    if (kind === 'velocity') return pat.steps[channelId] ?? new Array(STEPS_PER_PATTERN).fill(0)
    // Mod-X / Shift reuse the legacy `filterSteps` / `gateSteps`
    // fields so v0.169 saves keep loading. Note / Release / Mod-Y /
    // Rep live in their own *Steps fields.
    const fieldByKind: Record<Exclude<StepGraphKind, 'velocity'>, GraphKey> = {
      note: 'noteSteps',
      release: 'releaseSteps',
      pitch: 'pitchSteps',
      modX: 'filterSteps',
      modY: 'modYSteps',
      pan: 'panSteps',
      shift: 'gateSteps',
      rep: 'repSteps',
    }
    const key = fieldByKind[kind as Exclude<StepGraphKind, 'velocity'>]
    const def = STEP_GRAPH_DEFAULTS[kind as Exclude<StepGraphKind, 'velocity'>]
    return pat[key]?.[channelId] ?? new Array(STEPS_PER_PATTERN).fill(def)
  },

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
