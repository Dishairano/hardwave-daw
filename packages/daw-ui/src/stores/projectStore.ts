import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

interface ProjectInfo {
  name: string
  author: string
  sample_rate: number
  track_count: number
  bpm: number
}

interface ProjectState {
  filePath: string | null
  projectName: string
  dirty: boolean

  newProject: () => Promise<void>
  saveProject: (path?: string) => Promise<void>
  loadProject: (path: string) => Promise<void>
  getInfo: () => Promise<ProjectInfo>
  markDirty: () => void
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  filePath: null,
  projectName: 'Untitled',
  dirty: false,

  newProject: async () => {
    await invoke('new_project')
    set({ filePath: null, projectName: 'Untitled', dirty: false })
  },

  saveProject: async (path?: string) => {
    const savePath = path || get().filePath
    if (!savePath) {
      // Need to pick a path
      const { save } = await import('@tauri-apps/plugin-dialog')
      const selected = await save({
        filters: [{ name: 'Hardwave Project', extensions: ['hwp'] }],
        defaultPath: `${get().projectName}.hwp`,
      })
      if (!selected) return
      await invoke('save_project', { path: selected })
      set({ filePath: selected, dirty: false })
    } else {
      await invoke('save_project', { path: savePath })
      set({ dirty: false })
    }
  },

  loadProject: async (path: string) => {
    await invoke('load_project', { path })
    const name = path.split('/').pop()?.replace('.hwp', '') || 'Untitled'
    set({ filePath: path, projectName: name, dirty: false })
  },

  getInfo: async () => {
    return await invoke<ProjectInfo>('get_project_info')
  },

  markDirty: () => set({ dirty: true }),
}))
