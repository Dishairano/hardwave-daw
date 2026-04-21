import { create } from 'zustand'

export interface TrackFolder {
  id: string
  name: string
  color: string
  collapsed: boolean
  trackIds: string[]
}

const STORAGE_KEY = 'hardwave.daw.trackFolders'
const DEFAULT_COLORS = ['#3B82F6', '#10B981', '#F59E0B', '#A855F7', '#EC4899', '#06B6D4']

function load(): TrackFolder[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed
      .filter(f =>
        typeof f === 'object' && f != null
        && typeof f.id === 'string'
        && typeof f.name === 'string'
        && typeof f.color === 'string'
        && typeof f.collapsed === 'boolean'
        && Array.isArray(f.trackIds)
        && f.trackIds.every((t: unknown) => typeof t === 'string'),
      )
      .map((f): TrackFolder => ({
        id: f.id, name: f.name, color: f.color, collapsed: f.collapsed,
        trackIds: [...f.trackIds],
      }))
  } catch { return [] }
}

function save(list: TrackFolder[]) {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(list)) } catch {}
}

interface FolderState {
  folders: TrackFolder[]
  createFolder: (trackIds: string[], name?: string, color?: string) => string
  deleteFolder: (id: string) => void
  renameFolder: (id: string, name: string) => void
  setFolderColor: (id: string, color: string) => void
  toggleCollapsed: (id: string) => void
  addTrackToFolder: (trackId: string, folderId: string) => void
  removeTrackFromFolder: (trackId: string) => void
  folderForTrack: (trackId: string) => TrackFolder | null
  isHidden: (trackId: string) => boolean
}

export const useTrackFolderStore = create<FolderState>((set, get) => ({
  folders: load(),
  createFolder: (trackIds, name, color) => {
    const id = `fld_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`
    const existing = get().folders
    const finalName = name && name.trim() ? name.trim() : `Folder ${existing.length + 1}`
    const finalColor = color || DEFAULT_COLORS[existing.length % DEFAULT_COLORS.length]
    const cleaned = [...new Set(trackIds)]
    const next = existing
      .map(f => ({ ...f, trackIds: f.trackIds.filter(t => !cleaned.includes(t)) }))
      .concat([{ id, name: finalName, color: finalColor, collapsed: false, trackIds: cleaned }])
    save(next)
    set({ folders: next })
    return id
  },
  deleteFolder: (id) => {
    const next = get().folders.filter(f => f.id !== id)
    save(next)
    set({ folders: next })
  },
  renameFolder: (id, name) => {
    const trimmed = name.trim()
    if (!trimmed) return
    const next = get().folders.map(f => f.id === id ? { ...f, name: trimmed } : f)
    save(next)
    set({ folders: next })
  },
  setFolderColor: (id, color) => {
    const next = get().folders.map(f => f.id === id ? { ...f, color } : f)
    save(next)
    set({ folders: next })
  },
  toggleCollapsed: (id) => {
    const next = get().folders.map(f => f.id === id ? { ...f, collapsed: !f.collapsed } : f)
    save(next)
    set({ folders: next })
  },
  addTrackToFolder: (trackId, folderId) => {
    const next = get().folders.map(f => {
      if (f.id === folderId) {
        if (f.trackIds.includes(trackId)) return f
        return { ...f, trackIds: [...f.trackIds, trackId] }
      }
      return { ...f, trackIds: f.trackIds.filter(t => t !== trackId) }
    })
    save(next)
    set({ folders: next })
  },
  removeTrackFromFolder: (trackId) => {
    const next = get().folders
      .map(f => ({ ...f, trackIds: f.trackIds.filter(t => t !== trackId) }))
      .filter(f => f.trackIds.length > 0)
    save(next)
    set({ folders: next })
  },
  folderForTrack: (trackId) => {
    return get().folders.find(f => f.trackIds.includes(trackId)) ?? null
  },
  isHidden: (trackId) => {
    const f = get().folders.find(f => f.trackIds.includes(trackId))
    return !!f && f.collapsed
  },
}))
