import { create } from 'zustand'
import { getCurrentWebview } from '@tauri-apps/api/webview'

const STORAGE_KEY = 'hardwave.daw.uiPreferences'

export const UI_SCALE_OPTIONS = [100, 125, 150, 175, 200] as const
export type UiScale = typeof UI_SCALE_OPTIONS[number]
export type UiScaleMode = 'auto' | UiScale

interface StoredPrefs {
  uiScale?: number
  uiScaleMode?: UiScaleMode
}

interface UiPreferencesState {
  /** Last explicitly-picked scale (used when `mode` is a fixed number). */
  uiScale: UiScale
  /** 'auto' follows the system's display scale, else a fixed scale. */
  mode: UiScaleMode
  /** Scale currently applied to the root. Matches uiScale unless mode === 'auto'. */
  effectiveScale: UiScale
  setUiScale: (scale: UiScale) => void
  setUiScaleMode: (mode: UiScaleMode) => void
}

/**
 * Auto is the system's own scale. Windows already enlarges the page by its
 * display scale (devicePixelRatio), so auto used to scale it a second time:
 * at 125 % a laptop got the interface at 156 %, with a quarter of the
 * window cut off on the right and at the bottom.
 */
function deriveAutoScale(): UiScale {
  return 100
}

function hydrate(): { uiScale: UiScale; mode: UiScaleMode } {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as StoredPrefs
      const uiScale = (parsed.uiScale && (UI_SCALE_OPTIONS as readonly number[]).includes(parsed.uiScale))
        ? parsed.uiScale as UiScale
        : 100
      const mode: UiScaleMode = parsed.uiScaleMode === 'auto'
        ? 'auto'
        : (parsed.uiScaleMode && (UI_SCALE_OPTIONS as readonly number[]).includes(parsed.uiScaleMode as number))
          ? parsed.uiScaleMode as UiScale
          : uiScale
      return { uiScale, mode }
    }
  } catch {
    /* ignore */
  }
  // First launch → default to 'auto' so high-DPI displays get a sensible scale.
  return { uiScale: 100, mode: 'auto' }
}

function persist(prefs: StoredPrefs) {
  try {
    const existing = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}')
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...existing, ...prefs }))
  } catch {
    /* ignore */
  }
}

/**
 * Scale the whole page the way browser zoom does, through the webview.
 * It used to be CSS `zoom` on the root, which leaves the viewport the same
 * size: everything sized to the window (100vw, 90vh, innerWidth) came out
 * too big by the scale, so the app ran off the screen at anything but 100 %.
 */
function applyScale(scale: UiScale) {
  try {
    getCurrentWebview().setZoom(scale / 100).catch(() => { /* not in the app */ })
  } catch {
    /* not in the app: a browser preview has no webview to zoom */
  }
}

function resolveScale(mode: UiScaleMode, _fixed: UiScale): UiScale {
  return mode === 'auto' ? deriveAutoScale() : mode
}

export const useUiPreferencesStore = create<UiPreferencesState>((set, get) => {
  const initial = hydrate()
  const initialEffective = resolveScale(initial.mode, initial.uiScale)
  return {
    uiScale: initial.uiScale,
    mode: initial.mode,
    effectiveScale: initialEffective,

    setUiScale: (scale) => {
      persist({ uiScale: scale, uiScaleMode: scale })
      applyScale(scale)
      set({ uiScale: scale, mode: scale, effectiveScale: scale })
    },

    setUiScaleMode: (mode) => {
      if (mode === get().mode) return
      const effective = resolveScale(mode, get().uiScale)
      persist(mode === 'auto' ? { uiScaleMode: 'auto' } : { uiScale: mode, uiScaleMode: mode })
      applyScale(effective)
      set(prev => ({
        mode,
        effectiveScale: effective,
        uiScale: mode === 'auto' ? prev.uiScale : mode,
      }))
    },
  }
})

if (typeof window !== 'undefined') {
  const { mode, uiScale } = hydrate()
  applyScale(resolveScale(mode, uiScale))
}
