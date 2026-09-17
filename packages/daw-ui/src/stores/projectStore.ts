import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { invokeOrToast } from '../api/invoke'
import { usePatternStore } from './patternStore'
import { useTempoMapStore } from './tempoMapStore'
import { hydrateTimelineState, resetTimelineState, serializeTimelineState } from './timelineState'

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
    // A new song starts with no markers and no punch range.
    resetTimelineState()
    await useTempoMapStore.getState().refresh()
    set({ filePath: null, projectName: 'Untitled', dirty: false })
  },

  saveProject: async (path?: string) => {
    await invoke('set_channel_rack_state', { payload: usePatternStore.getState().serialize() })
    await invoke('set_timeline_state', { payload: serializeTimelineState() })
    const savePath = path || get().filePath
    if (!savePath) {
      const { save } = await import('@tauri-apps/plugin-dialog')
      const selected = await save({
        filters: [{ name: 'Hardwave Project', extensions: ['hwp'] }],
        defaultPath: `${get().projectName}.hwp`,
      })
      if (!selected) return
      await invokeOrToast('save_project', { path: selected }, { message: 'Could not save the project' })
      set({ filePath: selected, dirty: false })
      get().pushRecent(selected)
    } else {
      await invokeOrToast('save_project', { path: savePath }, { message: 'Could not save the project' })
      set({ dirty: false })
      get().pushRecent(savePath)
    }
  },

  loadProject: async (path: string) => {
    await invoke('load_project', { path })
    const rackState = await invoke<string | null>('get_channel_rack_state')
    usePatternStore.getState().hydrate(rackState)
    hydrateTimelineState(await invoke<string | null>('get_timeline_state'))
    // The playlist's bar grid comes from the loaded project's tempo map, so a
    // song in 7/8 draws in 7/8 from the moment it opens.
    await useTempoMapStore.getState().refresh()
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

/**
 * Tell the user which samples the project cannot find, and offer to find them.
 *
 * This used to HEAD-request `convertFileSrc(clip.source_id)`, but source_id is
 * the audio pool's id, not a path, so the request could never succeed: every
 * project load warned that its audio was missing and listed hashes where the
 * file names should be. The backend knows the real answer, so it is asked.
 */
async function reportMissingAudioSources() {
  const [{ useNotificationStore }, { invoke }] = await Promise.all([
    import('./notificationStore'),
    import('@tauri-apps/api/core'),
  ])

  interface MissingSource { sourceId: string; file: string; name: string; hash: string; clipCount: number }
  const missing = await invoke<MissingSource[]>('list_missing_sources').catch(() => [])
  if (missing.length === 0) return

  const { push } = useNotificationStore.getState()
  const preview = missing.slice(0, 4).map(m => `• ${m.name}`).join('\n')
  const more = missing.length > 4 ? `\n…and ${missing.length - 4} more` : ''

  // Everywhere worth looking without asking: the folders already added to the
  // browser, plus the folder the project itself lives in.
  const searchDirs = async (): Promise<string[]> => {
    const { useBrowserStore } = await import('./browserStore')
    const dirs = [...useBrowserStore.getState().diskRoots]
    const projectPath = useProjectStore.getState().filePath
    if (projectPath) {
      const dir = projectPath.slice(0, projectPath.lastIndexOf(projectPath.includes('\\') ? '\\' : '/'))
      if (dir) dirs.push(dir)
    }
    return dirs
  }

  const search = async () => {
    const dirs = await searchDirs()
    if (dirs.length === 0) {
      push('warning', 'Nowhere to search yet', {
        detail: 'Add your sample folders to the browser under Places, then try again.',
      })
      return
    }
    const found = await invoke<string[]>('auto_relink_sources', { dirs }).catch(() => [])
    if (found.length === 0) {
      push('warning', 'Could not find those samples', {
        detail: 'Nothing matching turned up in your sample folders. Use Locate to point at a file yourself.',
      })
      return
    }
    push('info', `Found ${found.length} of ${missing.length} missing sample${missing.length === 1 ? '' : 's'}`, {
      detail: found.slice(0, 6).join('\n'),
    })
    await useTrackStoreRefresh()
    void reportMissingAudioSources()
  }

  const locate = async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    for (const m of missing) {
      const picked = await open({
        title: `Locate ${m.name}`,
        multiple: false,
        filters: [{ name: 'Audio', extensions: ['wav', 'mp3', 'flac', 'aiff', 'aif', 'ogg', 'm4a'] }],
      })
      if (typeof picked !== 'string') break // cancelled: stop asking
      await invoke('relink_source', { sourceId: m.sourceId, newPath: picked }).catch(() => {})
    }
    await useTrackStoreRefresh()
    void reportMissingAudioSources()
  }

  push('warning',
    `${missing.length} audio file${missing.length === 1 ? '' : 's'} missing`,
    {
      detail: preview + more,
      sticky: true,
      actions: [
        { label: 'Search my folders', onClick: () => { void search() } },
        { label: 'Locate…', onClick: () => { void locate() } },
      ],
    },
  )
}

async function useTrackStoreRefresh() {
  const { useTrackStore } = await import('./trackStore')
  await useTrackStore.getState().fetchTracks()
}
