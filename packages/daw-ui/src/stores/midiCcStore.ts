import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

export const CC_LANE_RESOLUTION = 512
export const CC_LANE_DEFAULT_HEIGHT = 70

export interface CcLaneDefinition {
  id: string
  kind: 'pitchBend' | 'cc'
  cc?: number
  label: string
  shortLabel: string
}

export const BUILT_IN_CC_LANES: CcLaneDefinition[] = [
  { id: 'pb', kind: 'pitchBend', label: 'Pitch Bend', shortLabel: 'PB' },
  { id: 'cc1', kind: 'cc', cc: 1, label: 'Mod Wheel (CC1)', shortLabel: 'MOD' },
  { id: 'cc2', kind: 'cc', cc: 2, label: 'Breath (CC2)', shortLabel: 'BRE' },
  { id: 'cc11', kind: 'cc', cc: 11, label: 'Expression (CC11)', shortLabel: 'EXP' },
  { id: 'cc64', kind: 'cc', cc: 64, label: 'Sustain (CC64)', shortLabel: 'SUS' },
]

export function customCcLane(cc: number): CcLaneDefinition {
  const n = Math.max(0, Math.min(127, Math.floor(cc)))
  return { id: `cc${n}`, kind: 'cc', cc: n, label: `CC${n}`, shortLabel: `C${n}` }
}

export function laneDefaultValue(def: CcLaneDefinition): number {
  return def.kind === 'pitchBend' ? 0.5 : 0
}

/**
 * A lane's value is drawn as 0..1 with pitch bend centred at 0.5, while the
 * clip stores bend as -1..1, so the two conversions live here.
 */
function toClipValue(def: CcLaneDefinition, slotValue: number): number {
  return def.kind === 'pitchBend' ? slotValue * 2 - 1 : slotValue
}

function fromClipValue(def: CcLaneDefinition, clipValue: number): number {
  return def.kind === 'pitchBend' ? (clipValue + 1) / 2 : clipValue
}

/** The lane a stored id refers to, built-in or a plain CC number. */
export function laneDefinition(laneId: string): CcLaneDefinition | null {
  const built = BUILT_IN_CC_LANES.find(l => l.id === laneId)
  if (built) return built
  if (laneId.startsWith('cc')) {
    const n = parseInt(laneId.slice(2), 10)
    if (!Number.isNaN(n)) return customCcLane(n)
  }
  return null
}

function laneArgs(def: CcLaneDefinition) {
  return { kind: def.kind === 'pitchBend' ? 'pitchBend' : 'cc', cc: def.cc ?? null }
}

export function laneDisplayValue(def: CcLaneDefinition, normalized: number): string {
  if (def.kind === 'pitchBend') {
    const v = Math.round((normalized - 0.5) * 2 * 8191)
    return v > 0 ? `+${v}` : `${v}`
  }
  return String(Math.round(normalized * 127))
}

interface MidiCcState {
  values: Record<string, Record<string, number[]>>
  visibleLanes: Record<string, string[]>
  laneHeight: Record<string, number>

  getValues: (clipId: string, laneId: string) => number[]
  getVisibleLanes: (clipId: string) => string[]
  getLaneHeight: (clipId: string) => number
  addLane: (clipId: string, laneId: string) => void
  removeLane: (clipId: string, laneId: string) => void
  setValueAt: (clipId: string, def: CcLaneDefinition, slot: number, value: number) => void
  setValuesRange: (clipId: string, def: CcLaneDefinition, fromSlot: number, values: number[]) => void
  clearLane: (clipId: string, laneId: string) => void
  setLaneHeight: (clipId: string, h: number) => void

  serialize: () => string
  hydrate: (json: string | null) => void

  /**
   * Which clip the lanes are editing. Set by the piano roll, so an edit
   * knows which clip to write back into without every call passing it.
   */
  bind: (trackId: string | null, clipId: string | null, lengthTicks: number) => void
  loadLane: (trackId: string, clipId: string, def: CcLaneDefinition, lengthTicks: number) => Promise<void>
  flushLane: (trackId: string, clipId: string, def: CcLaneDefinition, lengthTicks: number) => Promise<void>
}

/**
 * Edits are written back a moment after the last one: drawing a curve is
 * hundreds of slot changes, and each would otherwise be a project edit, an
 * undo step and a graph rebuild.
 */
const FLUSH_DELAY_MS = 300
const flushTimers = new Map<string, number>()
let binding: { trackId: string; clipId: string; lengthTicks: number } | null = null

function scheduleFlush(clipId: string, laneId: string) {
  if (!binding || binding.clipId !== clipId) return
  const def = laneDefinition(laneId)
  if (!def) return
  const key = `${clipId}:${laneId}`
  const existing = flushTimers.get(key)
  if (existing !== undefined) window.clearTimeout(existing)
  const { trackId, lengthTicks } = binding
  flushTimers.set(key, window.setTimeout(() => {
    flushTimers.delete(key)
    void useMidiCcStore.getState().flushLane(trackId, clipId, def, lengthTicks)
  }, FLUSH_DELAY_MS))
}

function tickOfSlot(slot: number, lengthTicks: number): number {
  return Math.round((slot / (CC_LANE_RESOLUTION - 1)) * Math.max(0, lengthTicks))
}

function ensureArray(existing: number[] | undefined, def: CcLaneDefinition): number[] {
  if (existing && existing.length === CC_LANE_RESOLUTION) return existing
  const fill = laneDefaultValue(def)
  const arr = new Array(CC_LANE_RESOLUTION).fill(fill)
  if (existing) {
    for (let i = 0; i < Math.min(existing.length, CC_LANE_RESOLUTION); i++) arr[i] = existing[i]
  }
  return arr
}

export const useMidiCcStore = create<MidiCcState>((set, get) => ({
  values: {},
  visibleLanes: {},
  laneHeight: {},

  getValues: (clipId, laneId) => get().values[clipId]?.[laneId] ?? [],

  getVisibleLanes: (clipId) => get().visibleLanes[clipId] ?? [],

  getLaneHeight: (clipId) => get().laneHeight[clipId] ?? CC_LANE_DEFAULT_HEIGHT,

  addLane: (clipId, laneId) => set(s => {
    const list = s.visibleLanes[clipId] ?? []
    if (list.includes(laneId)) return {}
    return { visibleLanes: { ...s.visibleLanes, [clipId]: [...list, laneId] } }
  }),

  removeLane: (clipId, laneId) => set(s => {
    const list = s.visibleLanes[clipId] ?? []
    const next = list.filter(id => id !== laneId)
    return { visibleLanes: { ...s.visibleLanes, [clipId]: next } }
  }),

  setValueAt: (clipId, def, slot, value) => set(s => {
    if (slot < 0 || slot >= CC_LANE_RESOLUTION) return {}
    const clamped = Math.max(0, Math.min(1, value))
    const clipVals = { ...(s.values[clipId] ?? {}) }
    const arr = ensureArray(clipVals[def.id], def).slice()
    arr[slot] = clamped
    clipVals[def.id] = arr
    scheduleFlush(clipId, def.id)
    return { values: { ...s.values, [clipId]: clipVals } }
  }),

  setValuesRange: (clipId, def, fromSlot, values) => set(s => {
    const clipVals = { ...(s.values[clipId] ?? {}) }
    const arr = ensureArray(clipVals[def.id], def).slice()
    for (let i = 0; i < values.length; i++) {
      const idx = fromSlot + i
      if (idx < 0 || idx >= CC_LANE_RESOLUTION) continue
      arr[idx] = Math.max(0, Math.min(1, values[i]))
    }
    clipVals[def.id] = arr
    scheduleFlush(clipId, def.id)
    return { values: { ...s.values, [clipId]: clipVals } }
  }),

  clearLane: (clipId, laneId) => set(s => {
    const clipVals = { ...(s.values[clipId] ?? {}) }
    delete clipVals[laneId]
    scheduleFlush(clipId, laneId)
    return { values: { ...s.values, [clipId]: clipVals } }
  }),

  setLaneHeight: (clipId, h) => set(s => ({
    laneHeight: { ...s.laneHeight, [clipId]: Math.max(36, Math.min(260, h)) },
  })),

  serialize: () => {
    const { values, visibleLanes, laneHeight } = get()
    return JSON.stringify({ v: 1, values, visibleLanes, laneHeight })
  },

  bind: (trackId, clipId, lengthTicks) => {
    binding = trackId && clipId ? { trackId, clipId, lengthTicks } : null
  },

  /**
   * Read a lane out of the clip in the project. Points are held until the
   * next one, the way a controller behaves, so a recorded sweep fills the
   * lane instead of leaving gaps between its points.
   */
  loadLane: async (trackId, clipId, def, lengthTicks) => {
    let points: { tick: number; value: number }[]
    try {
      points = await invoke<{ tick: number; value: number }[]>('get_clip_controls', {
        trackId, clipId, ...laneArgs(def),
      })
    } catch {
      return
    }
    const arr = new Array(CC_LANE_RESOLUTION).fill(laneDefaultValue(def))
    let next = 0
    let held = points.length > 0 ? fromClipValue(def, points[0].value) : laneDefaultValue(def)
    for (let slot = 0; slot < CC_LANE_RESOLUTION; slot++) {
      const tick = tickOfSlot(slot, lengthTicks)
      while (next < points.length && points[next].tick <= tick) {
        held = fromClipValue(def, points[next].value)
        next++
      }
      arr[slot] = Math.max(0, Math.min(1, held))
    }
    set(s => ({ values: { ...s.values, [clipId]: { ...(s.values[clipId] ?? {}), [def.id]: arr } } }))
  },

  /**
   * Write a lane back into the clip. Only the slots where the value
   * changes become points, so a flat lane costs one point and a drawn
   * curve keeps its shape without 512 points per lane.
   */
  flushLane: async (trackId, clipId, def, lengthTicks) => {
    const arr = get().values[clipId]?.[def.id]
    const fallback = laneDefaultValue(def)
    const points: { tick: number; value: number }[] = []
    if (arr) {
      let previous: number | null = null
      for (let slot = 0; slot < arr.length; slot++) {
        const v = arr[slot] ?? fallback
        if (previous !== null && Math.abs(v - previous) < 0.0005) continue
        previous = v
        points.push({ tick: tickOfSlot(slot, lengthTicks), value: toClipValue(def, v) })
      }
      // A lane left at its default says nothing; do not store 1 point for it.
      if (points.length === 1 && Math.abs(points[0].value - toClipValue(def, fallback)) < 0.0005) {
        points.length = 0
      }
    }
    try {
      await invoke('set_clip_controls', { trackId, clipId, ...laneArgs(def), points })
    } catch (e) {
      console.warn('set_clip_controls failed', e)
    }
  },

  hydrate: (json) => {
    if (!json) { set({ values: {}, visibleLanes: {}, laneHeight: {} }); return }
    try {
      const p = JSON.parse(json)
      set({
        values: p.values ?? {},
        visibleLanes: p.visibleLanes ?? {},
        laneHeight: p.laneHeight ?? {},
      })
    } catch {
      set({ values: {}, visibleLanes: {}, laneHeight: {} })
    }
  },
}))
