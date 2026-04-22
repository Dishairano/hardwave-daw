import { create } from 'zustand'

const STORAGE_KEY = 'hardwave.daw.slotPresets'

export interface SlotPreset {
  id: string
  name: string
  createdAt: number
  pluginId: string
  pluginName: string
  wet: number
  enabled: boolean
}

interface SlotPresetState {
  presets: SlotPreset[]
  load: () => void
  add: (name: string, data: Omit<SlotPreset, 'id' | 'name' | 'createdAt'>) => SlotPreset
  rename: (id: string, name: string) => void
  remove: (id: string) => void
  get: (id: string) => SlotPreset | undefined
}

function persist(presets: SlotPreset[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(presets))
  } catch {
    /* ignore quota/serialization errors */
  }
}

function hydrate(): SlotPreset[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      p =>
        p &&
        typeof p.id === 'string' &&
        typeof p.name === 'string' &&
        typeof p.pluginId === 'string' &&
        typeof p.wet === 'number',
    )
  } catch {
    return []
  }
}

export const useSlotPresetStore = create<SlotPresetState>((set, get) => ({
  presets: [],

  load: () => set({ presets: hydrate() }),

  add: (name, data) => {
    const p: SlotPreset = {
      id: `sp_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`,
      name: name.trim() || 'Untitled Preset',
      createdAt: Date.now(),
      ...data,
    }
    const next = [...get().presets, p]
    persist(next)
    set({ presets: next })
    return p
  },

  rename: (id, name) => {
    const trimmed = name.trim()
    if (!trimmed) return
    const next = get().presets.map(p => (p.id === id ? { ...p, name: trimmed } : p))
    persist(next)
    set({ presets: next })
  },

  remove: (id) => {
    const next = get().presets.filter(p => p.id !== id)
    persist(next)
    set({ presets: next })
  },

  get: (id) => get().presets.find(p => p.id === id),
}))
