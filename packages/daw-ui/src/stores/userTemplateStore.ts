import { create } from 'zustand'

const STORAGE_KEY = 'hardwave.daw.userTemplates'

export interface UserTemplateTrack {
  name: string
  kind: string
  color: string
  volume_db: number
  pan: number
}

export interface UserTemplate {
  id: string
  name: string
  createdAt: number
  tracks: UserTemplateTrack[]
  bpm?: number
}

interface UserTemplateState {
  templates: UserTemplate[]
  load: () => void
  add: (name: string, tracks: UserTemplateTrack[], bpm?: number) => UserTemplate
  rename: (id: string, name: string) => void
  remove: (id: string) => void
  get: (id: string) => UserTemplate | undefined
}

function persist(templates: UserTemplate[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(templates))
  } catch {
    /* ignore quota/serialization errors */
  }
}

function hydrate(): UserTemplate[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      t => t && typeof t.id === 'string' && typeof t.name === 'string' && Array.isArray(t.tracks),
    )
  } catch {
    return []
  }
}

export const useUserTemplateStore = create<UserTemplateState>((set, get) => ({
  templates: [],

  load: () => set({ templates: hydrate() }),

  add: (name, tracks, bpm) => {
    const t: UserTemplate = {
      id: `ut_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`,
      name: name.trim() || 'Untitled Template',
      createdAt: Date.now(),
      tracks,
      bpm,
    }
    const next = [...get().templates, t]
    persist(next)
    set({ templates: next })
    return t
  },

  rename: (id, name) => {
    const trimmed = name.trim()
    if (!trimmed) return
    const next = get().templates.map(t => (t.id === id ? { ...t, name: trimmed } : t))
    persist(next)
    set({ templates: next })
  },

  remove: (id) => {
    const next = get().templates.filter(t => t.id !== id)
    persist(next)
    set({ templates: next })
  },

  get: (id) => get().templates.find(t => t.id === id),
}))
