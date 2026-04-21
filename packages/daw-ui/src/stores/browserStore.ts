import { create } from 'zustand'

const PLUGIN_FAV_KEY = 'hardwave.daw.pluginFavorites'
const FILE_FAV_KEY = 'hardwave.daw.fileFavorites'
const FILE_RECENT_KEY = 'hardwave.daw.fileRecents'
const FOLDERS_KEY = 'hardwave.daw.fileFolders'
const FILE_FOLDER_MAP_KEY = 'hardwave.daw.fileFolderMap'
const EXPANDED_FOLDERS_KEY = 'hardwave.daw.expandedFolders'
const FILE_TAGS_KEY = 'hardwave.daw.fileTags'
const RECENT_MAX = 25

function loadList(key: string): string[] {
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed.filter(x => typeof x === 'string') : []
  } catch { return [] }
}

function saveList(key: string, list: string[]) {
  try { localStorage.setItem(key, JSON.stringify(list)) } catch {}
}

function loadJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return fallback
    return JSON.parse(raw) as T
  } catch { return fallback }
}

function saveJson(key: string, value: unknown) {
  try { localStorage.setItem(key, JSON.stringify(value)) } catch {}
}

export interface FolderNode {
  id: string
  name: string
  parentId: string | null
}

function newFolderId() {
  return 'f_' + Math.random().toString(36).slice(2, 10) + Date.now().toString(36)
}

function isDescendantOf(folders: FolderNode[], candidateId: string, possibleAncestorId: string): boolean {
  let cur: string | null = candidateId
  const seen = new Set<string>()
  while (cur) {
    if (seen.has(cur)) return false
    seen.add(cur)
    if (cur === possibleAncestorId) return true
    const f = folders.find(x => x.id === cur)
    cur = f ? f.parentId : null
  }
  return false
}

interface BrowserState {
  pluginFavorites: Set<string>
  fileFavorites: Set<string>
  fileRecents: string[]

  folders: FolderNode[]
  fileFolderMap: Record<string, string | null>
  expandedFolders: Set<string>

  fileTags: Record<string, string[]>

  togglePluginFavorite: (id: string) => void
  toggleFileFavorite: (path: string) => void
  pushFileRecent: (path: string) => void
  removeFileRecent: (path: string) => void
  clearFileRecents: () => void

  createFolder: (name: string, parentId: string | null) => string
  renameFolder: (id: string, name: string) => void
  deleteFolder: (id: string) => void
  moveFolder: (id: string, newParentId: string | null) => void
  moveFile: (path: string, folderId: string | null) => void
  toggleFolderExpanded: (id: string) => void
  setFolderExpanded: (id: string, expanded: boolean) => void

  addFileTag: (path: string, tag: string) => void
  removeFileTag: (path: string, tag: string) => void
  clearFileTags: (path: string) => void
}

export const useBrowserStore = create<BrowserState>((set, get) => ({
  pluginFavorites: new Set(loadList(PLUGIN_FAV_KEY)),
  fileFavorites: new Set(loadList(FILE_FAV_KEY)),
  fileRecents: loadList(FILE_RECENT_KEY),

  folders: loadJson<FolderNode[]>(FOLDERS_KEY, []),
  fileFolderMap: loadJson<Record<string, string | null>>(FILE_FOLDER_MAP_KEY, {}),
  expandedFolders: new Set(loadList(EXPANDED_FOLDERS_KEY)),

  fileTags: loadJson<Record<string, string[]>>(FILE_TAGS_KEY, {}),

  togglePluginFavorite: (id) => {
    const next = new Set(get().pluginFavorites)
    if (next.has(id)) next.delete(id); else next.add(id)
    saveList(PLUGIN_FAV_KEY, Array.from(next))
    set({ pluginFavorites: next })
  },

  toggleFileFavorite: (path) => {
    const next = new Set(get().fileFavorites)
    if (next.has(path)) next.delete(path); else next.add(path)
    saveList(FILE_FAV_KEY, Array.from(next))
    set({ fileFavorites: next })
  },

  pushFileRecent: (path) => {
    const next = [path, ...get().fileRecents.filter(p => p !== path)].slice(0, RECENT_MAX)
    saveList(FILE_RECENT_KEY, next)
    set({ fileRecents: next })
  },

  removeFileRecent: (path) => {
    const next = get().fileRecents.filter(p => p !== path)
    saveList(FILE_RECENT_KEY, next)
    set({ fileRecents: next })
  },

  clearFileRecents: () => {
    saveList(FILE_RECENT_KEY, [])
    set({ fileRecents: [] })
  },

  createFolder: (name, parentId) => {
    const trimmed = (name || '').trim() || 'New Folder'
    const id = newFolderId()
    const next = [...get().folders, { id, name: trimmed, parentId }]
    saveJson(FOLDERS_KEY, next)
    const expanded = new Set(get().expandedFolders)
    if (parentId) {
      expanded.add(parentId)
      saveList(EXPANDED_FOLDERS_KEY, Array.from(expanded))
    }
    set({ folders: next, expandedFolders: expanded })
    return id
  },

  renameFolder: (id, name) => {
    const trimmed = (name || '').trim()
    if (!trimmed) return
    const next = get().folders.map(f => f.id === id ? { ...f, name: trimmed } : f)
    saveJson(FOLDERS_KEY, next)
    set({ folders: next })
  },

  deleteFolder: (id) => {
    const folders = get().folders
    const target = folders.find(f => f.id === id)
    if (!target) return
    const newParent = target.parentId
    const nextFolders = folders
      .filter(f => f.id !== id)
      .map(f => f.parentId === id ? { ...f, parentId: newParent } : f)
    const nextMap: Record<string, string | null> = {}
    for (const [path, folderId] of Object.entries(get().fileFolderMap)) {
      nextMap[path] = folderId === id ? newParent : folderId
    }
    const expanded = new Set(get().expandedFolders)
    expanded.delete(id)
    saveJson(FOLDERS_KEY, nextFolders)
    saveJson(FILE_FOLDER_MAP_KEY, nextMap)
    saveList(EXPANDED_FOLDERS_KEY, Array.from(expanded))
    set({ folders: nextFolders, fileFolderMap: nextMap, expandedFolders: expanded })
  },

  moveFolder: (id, newParentId) => {
    if (id === newParentId) return
    const folders = get().folders
    if (newParentId && isDescendantOf(folders, newParentId, id)) return
    const target = folders.find(f => f.id === id)
    if (!target || target.parentId === newParentId) return
    const next = folders.map(f => f.id === id ? { ...f, parentId: newParentId } : f)
    saveJson(FOLDERS_KEY, next)
    if (newParentId) {
      const expanded = new Set(get().expandedFolders)
      expanded.add(newParentId)
      saveList(EXPANDED_FOLDERS_KEY, Array.from(expanded))
      set({ folders: next, expandedFolders: expanded })
    } else {
      set({ folders: next })
    }
  },

  moveFile: (path, folderId) => {
    const next = { ...get().fileFolderMap, [path]: folderId }
    saveJson(FILE_FOLDER_MAP_KEY, next)
    if (folderId) {
      const expanded = new Set(get().expandedFolders)
      expanded.add(folderId)
      saveList(EXPANDED_FOLDERS_KEY, Array.from(expanded))
      set({ fileFolderMap: next, expandedFolders: expanded })
    } else {
      set({ fileFolderMap: next })
    }
  },

  toggleFolderExpanded: (id) => {
    const next = new Set(get().expandedFolders)
    if (next.has(id)) next.delete(id); else next.add(id)
    saveList(EXPANDED_FOLDERS_KEY, Array.from(next))
    set({ expandedFolders: next })
  },

  setFolderExpanded: (id, expanded) => {
    const next = new Set(get().expandedFolders)
    if (expanded) next.add(id); else next.delete(id)
    saveList(EXPANDED_FOLDERS_KEY, Array.from(next))
    set({ expandedFolders: next })
  },

  addFileTag: (path, tag) => {
    const trimmed = tag.trim().toLowerCase()
    if (!trimmed) return
    const current = get().fileTags[path] ?? []
    if (current.includes(trimmed)) return
    const next = { ...get().fileTags, [path]: [...current, trimmed] }
    saveJson(FILE_TAGS_KEY, next)
    set({ fileTags: next })
  },

  removeFileTag: (path, tag) => {
    const current = get().fileTags[path] ?? []
    const filtered = current.filter(t => t !== tag)
    const next = { ...get().fileTags }
    if (filtered.length === 0) delete next[path]
    else next[path] = filtered
    saveJson(FILE_TAGS_KEY, next)
    set({ fileTags: next })
  },

  clearFileTags: (path) => {
    if (!(path in get().fileTags)) return
    const next = { ...get().fileTags }
    delete next[path]
    saveJson(FILE_TAGS_KEY, next)
    set({ fileTags: next })
  },
}))
