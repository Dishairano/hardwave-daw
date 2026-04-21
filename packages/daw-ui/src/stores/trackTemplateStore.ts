import { create } from 'zustand'

const STORAGE_KEY = 'hardwave.daw.trackTemplates'

export interface TrackTemplate {
  id: string
  name: string
  kind: 'Audio' | 'Midi'
  trackName: string
  color: string
  volumeDb: number
  pan: number
  createdAt: number
}

interface TrackTemplateState {
  templates: TrackTemplate[]
  load: () => void
  save: (template: Omit<TrackTemplate, 'id' | 'createdAt'>) => TrackTemplate
  remove: (id: string) => void
  rename: (id: string, name: string) => void
  get: (id: string) => TrackTemplate | undefined
}

function persist(templates: TrackTemplate[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(templates))
  } catch {
    /* ignore quota errors */
  }
}

function hydrate(): TrackTemplate[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      (t): t is TrackTemplate =>
        t
        && typeof t.id === 'string'
        && typeof t.name === 'string'
        && (t.kind === 'Audio' || t.kind === 'Midi')
        && typeof t.trackName === 'string'
        && typeof t.color === 'string'
        && typeof t.volumeDb === 'number'
        && typeof t.pan === 'number',
    )
  } catch {
    return []
  }
}

export const useTrackTemplateStore = create<TrackTemplateState>((set, get) => ({
  templates: hydrate(),

  load: () => set({ templates: hydrate() }),

  save: (template) => {
    const created: TrackTemplate = {
      ...template,
      id: `tt_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`,
      createdAt: Date.now(),
    }
    const next = [...get().templates, created]
    persist(next)
    set({ templates: next })
    return created
  },

  remove: (id) => {
    const next = get().templates.filter(t => t.id !== id)
    persist(next)
    set({ templates: next })
  },

  rename: (id, name) => {
    const next = get().templates.map(t => t.id === id ? { ...t, name } : t)
    persist(next)
    set({ templates: next })
  },

  get: (id) => get().templates.find(t => t.id === id),
}))
