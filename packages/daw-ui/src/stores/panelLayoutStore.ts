import { create } from 'zustand'

export type PanelId = 'browser' | 'channelRack' | 'pianoRoll' | 'mixer' | 'playlist'

export interface PanelLayout {
  floating: boolean
  x: number
  y: number
  w: number
  h: number
  zIndex: number
}

const STORAGE_KEY = 'hardwave.daw.panelLayout'

const DEFAULT_LAYOUT: Record<PanelId, PanelLayout> = {
  browser:     { floating: false, x: 80,  y: 120, w: 280, h: 520, zIndex: 10 },
  channelRack: { floating: false, x: 200, y: 200, w: 640, h: 360, zIndex: 11 },
  pianoRoll:   { floating: false, x: 240, y: 240, w: 720, h: 420, zIndex: 12 },
  mixer:       { floating: false, x: 120, y: 300, w: 780, h: 300, zIndex: 13 },
  playlist:    { floating: false, x: 160, y: 160, w: 820, h: 440, zIndex: 14 },
}

function loadLayout(): Record<PanelId, PanelLayout> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return { ...DEFAULT_LAYOUT }
    const parsed = JSON.parse(raw)
    const out: Record<PanelId, PanelLayout> = { ...DEFAULT_LAYOUT }
    for (const key of Object.keys(DEFAULT_LAYOUT) as PanelId[]) {
      if (parsed && parsed[key]) out[key] = { ...DEFAULT_LAYOUT[key], ...parsed[key] }
    }
    return out
  } catch { return { ...DEFAULT_LAYOUT } }
}

function saveLayout(layout: Record<PanelId, PanelLayout>) {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(layout)) } catch {}
}

interface PanelLayoutState {
  layout: Record<PanelId, PanelLayout>
  topZ: number
  toggleFloat: (id: PanelId) => void
  setFloating: (id: PanelId, floating: boolean) => void
  setPos: (id: PanelId, x: number, y: number) => void
  setSize: (id: PanelId, w: number, h: number) => void
  bringToFront: (id: PanelId) => void
}

export const usePanelLayoutStore = create<PanelLayoutState>((set, get) => ({
  layout: loadLayout(),
  topZ: 20,

  toggleFloat: (id) => {
    const layout = { ...get().layout }
    layout[id] = { ...layout[id], floating: !layout[id].floating }
    saveLayout(layout)
    set({ layout })
  },

  setFloating: (id, floating) => {
    const layout = { ...get().layout }
    if (layout[id].floating === floating) return
    layout[id] = { ...layout[id], floating }
    saveLayout(layout)
    set({ layout })
  },

  setPos: (id, x, y) => {
    const layout = { ...get().layout }
    layout[id] = { ...layout[id], x, y }
    saveLayout(layout)
    set({ layout })
  },

  setSize: (id, w, h) => {
    const layout = { ...get().layout }
    layout[id] = { ...layout[id], w, h }
    saveLayout(layout)
    set({ layout })
  },

  bringToFront: (id) => {
    const topZ = get().topZ + 1
    const layout = { ...get().layout }
    layout[id] = { ...layout[id], zIndex: topZ }
    saveLayout(layout)
    set({ layout, topZ })
  },
}))
