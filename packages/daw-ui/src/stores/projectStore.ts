import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

interface ProjectInfo {
  name: string
  author: string
  sample_rate: number
  track_count: number
  bpm: number
}

const RECENT_KEY = 'hardwave.daw.recentProjects'
const RECENT_MAX = 10

function loadRecent(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed.filter(x => typeof x === 'string') : []
  } catch { return [] }
}

function saveRecent(list: string[]) {
  try { localStorage.setItem(RECENT_KEY, JSON.stringify(list)) } catch {}
}

interface ProjectState {
  filePath: string | null
  projectName: string
  dirty: boolean
  recentProjects: string[]

  newProject: () => Promise<void>
  saveProject: (path?: string) => Promise<void>
  loadProject: (path: string) => Promise<void>
  getInfo: () => Promise<ProjectInfo>
  markDirty: () => void
  pushRecent: (path: string) => void
  clearRecent: () => void
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  filePath: null,
  projectName: 'Untitled',
  dirty: false,
  recentProjects: loadRecent(),

  newProject: async () => {
    await invoke('new_project')
    set({ filePath: null, projectName: 'Untitled', dirty: false })
  },

  saveProject: async (path?: string) => {
    const savePath = path || get().filePath
    if (!savePath) {
      const { save } = await import('@tauri-apps/plugin-dialog')
      const selected = await save({
        filters: [{ name: 'Hardwave Project', extensions: ['hwp'] }],
        defaultPath: `${get().projectName}.hwp`,
      })
      if (!selected) return
      await invoke('save_project', { path: selected })
      set({ filePath: selected, dirty: false })
      get().pushRecent(selected)
    } else {
      await invoke('save_project', { path: savePath })
      set({ dirty: false })
      get().pushRecent(savePath)
    }
  },

  loadProject: async (path: string) => {
    await invoke('load_project', { path })
    const name = path.split(/[\\/]/).pop()?.replace('.hwp', '') || 'Untitled'
    set({ filePath: path, projectName: name, dirty: false })
    get().pushRecent(path)
  },

  getInfo: async () => {
    return await invoke<ProjectInfo>('get_project_info')
  },

  markDirty: () => set({ dirty: true }),

  pushRecent: (path: string) => {
    const next = [path, ...get().recentProjects.filter(p => p !== path)].slice(0, RECENT_MAX)
    saveRecent(next)
    set({ recentProjects: next })
  },

  clearRecent: () => {
    saveRecent([])
    set({ recentProjects: [] })
  },
}))
