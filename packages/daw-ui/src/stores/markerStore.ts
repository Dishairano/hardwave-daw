import { create } from 'zustand'

export type MarkerKind = 'generic' | 'tempo' | 'timesig'

export interface Marker {
  id: string
  tick: number
  label: string
  color: string
  kind?: MarkerKind
  bpm?: number
  timeSigNum?: number
  timeSigDen?: number
}

const STORAGE_KEY = 'hardwave.daw.markers'

function load(): Marker[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(m =>
      typeof m === 'object' && m != null
      && typeof m.id === 'string'
      && typeof m.tick === 'number'
      && typeof m.label === 'string'
      && typeof m.color === 'string'
    ).map((m): Marker => ({
      id: m.id,
      tick: m.tick,
      label: m.label,
      color: m.color,
      kind: m.kind === 'tempo' || m.kind === 'timesig' ? m.kind : 'generic',
      bpm: typeof m.bpm === 'number' ? m.bpm : undefined,
      timeSigNum: typeof m.timeSigNum === 'number' ? m.timeSigNum : undefined,
      timeSigDen: typeof m.timeSigDen === 'number' ? m.timeSigDen : undefined,
    }))
  } catch { return [] }
}

function save(list: Marker[]) {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(list)) } catch {}
}

const DEFAULT_COLORS = ['#3B82F6', '#10B981', '#F59E0B', '#A855F7', '#EC4899', '#06B6D4']
const TEMPO_COLOR = '#F97316'
const TIMESIG_COLOR = '#14B8A6'

interface MarkerState {
  markers: Marker[]
  addMarker: (tick: number, label?: string, color?: string) => string
  addTempoMarker: (tick: number, bpm: number) => string
  addTimeSigMarker: (tick: number, num: number, den: number) => string
  removeMarker: (id: string) => void
  updateMarker: (id: string, patch: Partial<Omit<Marker, 'id'>>) => void
  clearMarkers: () => void
  jumpToNext: (currentTick: number) => Marker | null
  jumpToPrev: (currentTick: number) => Marker | null
}

export const useMarkerStore = create<MarkerState>((set, get) => ({
  markers: load(),
  addMarker: (tick, label, color) => {
    const id = `mk_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`
    const existing = get().markers
    const name = label && label.trim() ? label.trim() : `Marker ${existing.length + 1}`
    const col = color || DEFAULT_COLORS[existing.length % DEFAULT_COLORS.length]
    const next = [...existing, { id, tick: Math.max(0, Math.floor(tick)), label: name, color: col, kind: 'generic' as const }]
    next.sort((a, b) => a.tick - b.tick)
    save(next)
    set({ markers: next })
    return id
  },
  addTempoMarker: (tick, bpm) => {
    const id = `mk_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`
    const clampedBpm = Math.max(20, Math.min(999, bpm))
    const next = [...get().markers, {
      id, tick: Math.max(0, Math.floor(tick)),
      label: `${clampedBpm.toFixed(clampedBpm % 1 === 0 ? 0 : 2)} BPM`,
      color: TEMPO_COLOR, kind: 'tempo' as const, bpm: clampedBpm,
    }]
    next.sort((a, b) => a.tick - b.tick)
    save(next)
    set({ markers: next })
    return id
  },
  addTimeSigMarker: (tick, num, den) => {
    const id = `mk_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`
    const n = Math.max(1, Math.min(32, Math.floor(num)))
    const d = [1, 2, 4, 8, 16, 32].includes(den) ? den : 4
    const next = [...get().markers, {
      id, tick: Math.max(0, Math.floor(tick)),
      label: `${n}/${d}`,
      color: TIMESIG_COLOR, kind: 'timesig' as const, timeSigNum: n, timeSigDen: d,
    }]
    next.sort((a, b) => a.tick - b.tick)
    save(next)
    set({ markers: next })
    return id
  },
  removeMarker: (id) => {
    const next = get().markers.filter(m => m.id !== id)
    save(next)
    set({ markers: next })
  },
  updateMarker: (id, patch) => {
    const next = get().markers.map(m => m.id === id ? { ...m, ...patch, tick: patch.tick != null ? Math.max(0, Math.floor(patch.tick)) : m.tick } : m)
    next.sort((a, b) => a.tick - b.tick)
    save(next)
    set({ markers: next })
  },
  clearMarkers: () => { save([]); set({ markers: [] }) },
  jumpToNext: (currentTick) => {
    const list = get().markers
    return list.find(m => m.tick > currentTick) ?? null
  },
  jumpToPrev: (currentTick) => {
    const list = get().markers
    const earlier = list.filter(m => m.tick < currentTick)
    return earlier.length > 0 ? earlier[earlier.length - 1] : null
  },
}))
