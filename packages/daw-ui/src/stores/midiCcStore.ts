import { create } from 'zustand'

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
    return { values: { ...s.values, [clipId]: clipVals } }
  }),

  clearLane: (clipId, laneId) => set(s => {
    const clipVals = { ...(s.values[clipId] ?? {}) }
    delete clipVals[laneId]
    return { values: { ...s.values, [clipId]: clipVals } }
  }),

  setLaneHeight: (clipId, h) => set(s => ({
    laneHeight: { ...s.laneHeight, [clipId]: Math.max(36, Math.min(260, h)) },
  })),

  serialize: () => {
    const { values, visibleLanes, laneHeight } = get()
    return JSON.stringify({ v: 1, values, visibleLanes, laneHeight })
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
