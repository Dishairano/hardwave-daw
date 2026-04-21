import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { usePatternStore } from './patternStore'

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
  removeRecent: (path: string) => void
  clearRecent: () => void
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  filePath: null,
  projectName: 'Untitled',
  dirty: false,
  recentProjects: loadRecent(),

  newProject: async () => {
    await invoke('new_project')
    usePatternStore.getState().hydrate(null)
    await invoke('set_channel_rack_state', { payload: null })
    set({ filePath: null, projectName: 'Untitled', dirty: false })
  },

  saveProject: async (path?: string) => {
    await invoke('set_channel_rack_state', { payload: usePatternStore.getState().serialize() })
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
    const rackState = await invoke<string | null>('get_channel_rack_state')
    usePatternStore.getState().hydrate(rackState)
    const name = path.split(/[\\/]/).pop()?.replace('.hwp', '') || 'Untitled'
    set({ filePath: path, projectName: name, dirty: false })
    get().pushRecent(path)
    reportMissingAudioSources().catch(() => {})
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

  removeRecent: (path: string) => {
    const next = get().recentProjects.filter(p => p !== path)
    saveRecent(next)
    set({ recentProjects: next })
  },

  clearRecent: () => {
    saveRecent([])
    set({ recentProjects: [] })
  },
}))

// Scan current project's audio clips and warn about sources that can't be read.
// Runs HEAD fetches through convertFileSrc — no backend changes required.
async function reportMissingAudioSources() {
  const [{ useTrackStore }, { useNotificationStore }, { convertFileSrc }] = await Promise.all([
    import('./trackStore'),
    import('./notificationStore'),
    import('@tauri-apps/api/core'),
  ])
  await useTrackStore.getState().fetchTracks()
  const tracks = useTrackStore.getState().tracks
  const seen = new Set<string>()
  const candidates: string[] = []
  for (const t of tracks) {
    if (t.kind !== 'Audio') continue
    for (const c of t.clips) {
      if (c.kind !== 'audio') continue
      const p = c.source_id
      if (!p || seen.has(p)) continue
      seen.add(p)
      candidates.push(p)
    }
  }
  if (candidates.length === 0) return
  const checks = await Promise.all(candidates.map(async (p) => {
    try {
      const resp = await fetch(convertFileSrc(p), { method: 'HEAD' })
      return resp.ok ? null : p
    } catch {
      return p
    }
  }))
  const missing = checks.filter((x): x is string => x !== null)
  if (missing.length === 0) return
  const { push } = useNotificationStore.getState()
  const preview = missing.slice(0, 4).map(p => `• ${p.split(/[\\/]/).pop() || p}`).join('\n')
  const more = missing.length > 4 ? `\n…and ${missing.length - 4} more` : ''
  push('warning',
    `${missing.length} audio file${missing.length === 1 ? '' : 's'} missing`,
    { detail: preview + more, sticky: true },
  )
}
