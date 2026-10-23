import { create } from 'zustand'

export interface TrackFolder {
  id: string
  name: string
  color: string
  collapsed: boolean
  trackIds: string[]
}

/** Where folders lived before they were kept with the song. */
export const LEGACY_FOLDERS_KEY = 'hardwave.daw.trackFolders'
const DEFAULT_COLORS = ['#3B82F6', '#10B981', '#F59E0B', '#A855F7', '#EC4899', '#06B6D4']

/** Accepts anything; returns only the folders that are structurally sound. */
export function parseFolders(raw: unknown): TrackFolder[] {
  if (!Array.isArray(raw)) return []
  return raw
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
}

interface FolderState {
  folders: TrackFolder[]
  /** Replace the folders: a song was opened or a new one started. */
  setFolders: (folders: TrackFolder[]) => void
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
  // Folders belong to the song (see timelineState.ts). They lived in
  // localStorage, so they belonged to the machine: every song showed the
  // folders of the last one, since track ids repeat from song to song.
  folders: [],
  setFolders: (folders) => set({ folders }),
  createFolder: (trackIds, name, color) => {
    const id = `fld_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`
    const existing = get().folders
    const finalName = name && name.trim() ? name.trim() : `Folder ${existing.length + 1}`
    const finalColor = color || DEFAULT_COLORS[existing.length % DEFAULT_COLORS.length]
    const cleaned = [...new Set(trackIds)]
    const next = existing
      .map(f => ({ ...f, trackIds: f.trackIds.filter(t => !cleaned.includes(t)) }))
      .concat([{ id, name: finalName, color: finalColor, collapsed: false, trackIds: cleaned }])
    set({ folders: next })
    return id
  },
  deleteFolder: (id) => {
    const next = get().folders.filter(f => f.id !== id)
    set({ folders: next })
  },
  renameFolder: (id, name) => {
    const trimmed = name.trim()
    if (!trimmed) return
    const next = get().folders.map(f => f.id === id ? { ...f, name: trimmed } : f)
    set({ folders: next })
  },
  setFolderColor: (id, color) => {
    const next = get().folders.map(f => f.id === id ? { ...f, color } : f)
    set({ folders: next })
  },
  toggleCollapsed: (id) => {
    const next = get().folders.map(f => f.id === id ? { ...f, collapsed: !f.collapsed } : f)
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
    set({ folders: next })
  },
  removeTrackFromFolder: (trackId) => {
    const next = get().folders
      .map(f => ({ ...f, trackIds: f.trackIds.filter(t => t !== trackId) }))
      .filter(f => f.trackIds.length > 0)
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
