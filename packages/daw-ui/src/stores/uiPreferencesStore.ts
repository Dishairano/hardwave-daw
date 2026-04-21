import { create } from 'zustand'

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
  /** 'auto' → derived from devicePixelRatio, else a fixed scale. */
  mode: UiScaleMode
  /** Scale currently applied to the root. Matches uiScale unless mode === 'auto'. */
  effectiveScale: UiScale
  setUiScale: (scale: UiScale) => void
  setUiScaleMode: (mode: UiScaleMode) => void
}

function deriveAutoScale(): UiScale {
  const dpr = typeof window === 'undefined' ? 1 : (window.devicePixelRatio || 1)
  if (dpr >= 2.0) return 200
  if (dpr >= 1.75) return 175
  if (dpr >= 1.5) return 150
  if (dpr >= 1.25) return 125
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

function applyScale(scale: UiScale) {
  const root = document.documentElement as HTMLElement & { style: CSSStyleDeclaration & { zoom?: string } }
  root.style.zoom = String(scale / 100)
}

function resolveScale(mode: UiScaleMode, fixed: UiScale): UiScale {
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

  // Per-monitor DPI awareness: listen for DPR changes (user moves window to a different monitor,
  // system scale changes, etc.) and re-apply when in auto mode.
  // matchMedia on the current DPR fires when DPR moves off that value.
  let mq: MediaQueryList | null = null
  const rewire = () => {
    if (mq) mq.removeEventListener('change', onChange)
    const dpr = window.devicePixelRatio || 1
    mq = window.matchMedia(`(resolution: ${dpr}dppx)`)
    mq.addEventListener('change', onChange)
  }
  const onChange = () => {
    const state = useUiPreferencesStore.getState()
    if (state.mode === 'auto') {
      const next = deriveAutoScale()
      if (next !== state.effectiveScale) {
        applyScale(next)
        useUiPreferencesStore.setState({ effectiveScale: next })
      }
    }
    rewire()
  }
  rewire()
}
