import { create } from 'zustand'
import {
  dockRect, fitToArea, presetGeometry, presetShows, PANEL_KEYS,
  type Area, type DockSide, type LayoutName, type PanelGeometry, type PanelKey, type Rect,
} from '../lib/workspaceLayout'

/**
 * Where each panel sits in the work area, remembered on this computer.
 * Whether a panel is shown stays with the panel toggles (F5 to F9 and the
 * View menu); this only holds the places. The maths is lib/workspaceLayout.
 */

const STORAGE_KEY = 'hardwave.daw.workspace.v1'
const MINE_KEY = 'hardwave.daw.workspace.mine.v1'

interface Saved {
  layoutName: LayoutName
  area: Area
  panels: Record<PanelKey, PanelGeometry>
  /** For the own layout: which panels it shows. */
  shown?: PanelKey[]
}

function read<T>(key: string): T | null {
  try {
    const raw = localStorage.getItem(key)
    return raw ? (JSON.parse(raw) as T) : null
  } catch { return null }
}

function write(key: string, value: unknown) {
  try { localStorage.setItem(key, JSON.stringify(value)) } catch { /* storage off: the layout lasts the session */ }
}

function valid(s: Saved | null): s is Saved {
  return !!s && !!s.area && !!s.panels && PANEL_KEYS.every((k) => s.panels[k] && Number.isFinite(s.panels[k].w))
}

interface WorkspaceState {
  /** The work area the places were last fitted to; null until measured. */
  area: Area | null
  layoutName: LayoutName
  panels: Record<PanelKey, PanelGeometry> | null
  topZ: number
  /** The work area was measured (first time, or it changed size). */
  setArea: (area: Area) => void
  setRect: (key: PanelKey, rect: Rect, dock?: DockSide | null) => void
  front: (key: PanelKey) => void
  toggleMax: (key: PanelKey) => void
  dock: (key: PanelKey, side: DockSide) => void
  float: (key: PanelKey) => void
  /** Switch to a ready layout (or the saved own one) and say which panels
   *  it shows; null for "mine" when none was saved yet. */
  applyLayout: (name: LayoutName) => PanelKey[] | null
  /** Keep the current places, and which panels are shown, as the own layout. */
  saveMine: (shown: PanelKey[]) => void
  hasMine: () => boolean
}

function persist(s: Pick<WorkspaceState, 'area' | 'layoutName' | 'panels'>) {
  if (s.area && s.panels) write(STORAGE_KEY, { layoutName: s.layoutName, area: s.area, panels: s.panels } satisfies Saved)
}

export const useWorkspaceStore = create<WorkspaceState>((set, get) => {
  const saved = read<Saved>(STORAGE_KEY)
  const start = valid(saved) ? saved : null
  const update = (panels: Record<PanelKey, PanelGeometry>, extra: Partial<WorkspaceState> = {}) => {
    set({ panels, ...extra })
    persist({ ...get(), panels, ...extra })
  }
  return {
    area: start?.area ?? null,
    layoutName: start?.layoutName ?? 'default',
    panels: start?.panels ?? null,
    topZ: start ? Math.max(...PANEL_KEYS.map((k) => start.panels[k].z)) + 1 : 10,

    setArea: (area) => {
      const { panels, area: before, layoutName } = get()
      if (area.w < 50 || area.h < 50) return
      if (!panels || !before) {
        update(presetGeometry('default', area), { area, layoutName: layoutName === 'mine' ? 'default' : layoutName })
        return
      }
      if (before.w === area.w && before.h === area.h) return
      const next = { ...panels }
      for (const k of PANEL_KEYS) next[k] = fitToArea(panels[k], k, before, area)
      update(next, { area })
    },

    setRect: (key, rect, dock = null) => {
      const { panels } = get()
      if (!panels) return
      update({ ...panels, [key]: { ...panels[key], ...rect, dock, max: false } })
    },

    front: (key) => {
      const { panels, topZ } = get()
      if (!panels || panels[key].z === topZ - 1) return
      update({ ...panels, [key]: { ...panels[key], z: topZ } }, { topZ: topZ + 1 })
    },

    toggleMax: (key) => {
      const { panels, topZ } = get()
      if (!panels) return
      update({ ...panels, [key]: { ...panels[key], max: !panels[key].max, z: topZ } }, { topZ: topZ + 1 })
    },

    dock: (key, side) => {
      const { panels, area, topZ } = get()
      if (!panels || !area) return
      const r = dockRect(side, panels[key], area)
      update({ ...panels, [key]: { ...panels[key], ...r, dock: side, max: false, z: topZ } }, { topZ: topZ + 1 })
    },

    float: (key) => {
      const { panels, area } = get()
      if (!panels || !area) return
      const p = panels[key]
      const w = Math.round(Math.min(p.w, area.w * 0.6)), h = Math.round(Math.min(p.h, area.h * 0.6))
      update({ ...panels, [key]: { ...p, w, h, x: Math.round((area.w - w) / 2), y: Math.round((area.h - h) / 3), dock: null, max: false } })
    },

    applyLayout: (name) => {
      const { area, panels } = get()
      if (!area) return null
      if (name === 'mine') {
        const mine = read<Saved>(MINE_KEY)
        if (!valid(mine)) return null
        const next = { ...mine.panels }
        for (const k of PANEL_KEYS) next[k] = fitToArea(mine.panels[k], k, mine.area, area)
        update(next, { layoutName: 'mine' })
        return mine.shown ?? PANEL_KEYS
      }
      update(presetGeometry(name, area, panels ?? undefined), { layoutName: name })
      return presetShows(name)
    },

    saveMine: (shown) => {
      const { area, panels } = get()
      if (!area || !panels) return
      write(MINE_KEY, { layoutName: 'mine', area, panels, shown } satisfies Saved)
      set({ layoutName: 'mine' })
      persist({ ...get(), layoutName: 'mine' })
    },

    hasMine: () => valid(read<Saved>(MINE_KEY)),
  }
})
