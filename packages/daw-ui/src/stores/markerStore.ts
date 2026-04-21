import { create } from 'zustand'

export interface Marker {
  id: string
  tick: number
  label: string
  color: string
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
    )
  } catch { return [] }
}

function save(list: Marker[]) {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(list)) } catch {}
}

const DEFAULT_COLORS = ['#3B82F6', '#10B981', '#F59E0B', '#A855F7', '#EC4899', '#06B6D4']

interface MarkerState {
  markers: Marker[]
  addMarker: (tick: number, label?: string, color?: string) => string
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
    const next = [...existing, { id, tick: Math.max(0, Math.floor(tick)), label: name, color: col }]
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
