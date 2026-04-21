import { create } from 'zustand'

const STORAGE_KEY = 'hardwave.daw.clipGroups'

const GROUP_COLORS = [
  '#DC2626', '#F59E0B', '#10B981', '#06B6D4',
  '#3B82F6', '#A855F7', '#EC4899', '#84CC16',
]

interface Persisted {
  clipToGroup: Record<string, string>
  groupColors: Record<string, string>
}

function hydrate(): Persisted {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return { clipToGroup: {}, groupColors: {} }
    const parsed = JSON.parse(raw)
    const clipToGroup = (parsed && typeof parsed.clipToGroup === 'object' && parsed.clipToGroup) || {}
    const groupColors = (parsed && typeof parsed.groupColors === 'object' && parsed.groupColors) || {}
    return { clipToGroup, groupColors }
  } catch {
    return { clipToGroup: {}, groupColors: {} }
  }
}

function persist(state: Persisted) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state))
  } catch { /* ignore quota */ }
}

function newGroupId(): string {
  return `g_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`
}

function pickColor(used: string[]): string {
  for (const c of GROUP_COLORS) {
    if (!used.includes(c)) return c
  }
  return GROUP_COLORS[Math.floor(Math.random() * GROUP_COLORS.length)]
}

interface ClipGroupState {
  clipToGroup: Record<string, string>
  groupColors: Record<string, string>

  groupOf: (clipId: string) => string | null
  membersOf: (clipId: string) => string[]
  colorOf: (clipId: string) => string | null
  groupClips: (clipIds: string[]) => string | null
  ungroupClip: (clipId: string) => void
  ungroupGroup: (groupId: string) => void
}

export const useClipGroupStore = create<ClipGroupState>((set, get) => ({
  ...hydrate(),

  groupOf: (clipId) => get().clipToGroup[clipId] ?? null,

  membersOf: (clipId) => {
    const { clipToGroup } = get()
    const g = clipToGroup[clipId]
    if (!g) return [clipId]
    return Object.keys(clipToGroup).filter(k => clipToGroup[k] === g)
  },

  colorOf: (clipId) => {
    const { clipToGroup, groupColors } = get()
    const g = clipToGroup[clipId]
    if (!g) return null
    return groupColors[g] ?? null
  },

  groupClips: (clipIds) => {
    const unique = Array.from(new Set(clipIds)).filter(Boolean)
    if (unique.length < 2) return null
    const state = get()
    const clipToGroup = { ...state.clipToGroup }
    const groupColors = { ...state.groupColors }
    const gid = newGroupId()
    const used = Object.values(groupColors)
    groupColors[gid] = pickColor(used)
    for (const id of unique) clipToGroup[id] = gid
    persist({ clipToGroup, groupColors })
    set({ clipToGroup, groupColors })
    return gid
  },

  ungroupClip: (clipId) => {
    const state = get()
    const gid = state.clipToGroup[clipId]
    if (!gid) return
    const clipToGroup = { ...state.clipToGroup }
    delete clipToGroup[clipId]
    const remaining = Object.keys(clipToGroup).filter(k => clipToGroup[k] === gid)
    const groupColors = { ...state.groupColors }
    if (remaining.length <= 1) {
      for (const k of remaining) delete clipToGroup[k]
      delete groupColors[gid]
    }
    persist({ clipToGroup, groupColors })
    set({ clipToGroup, groupColors })
  },

  ungroupGroup: (groupId) => {
    const state = get()
    const clipToGroup = { ...state.clipToGroup }
    const groupColors = { ...state.groupColors }
    for (const k of Object.keys(clipToGroup)) {
      if (clipToGroup[k] === groupId) delete clipToGroup[k]
    }
    delete groupColors[groupId]
    persist({ clipToGroup, groupColors })
    set({ clipToGroup, groupColors })
  },
}))
