import { create } from 'zustand'

const STORAGE_KEY = 'hardwave.daw.uiPreferences'

export const UI_SCALE_OPTIONS = [100, 125, 150, 175, 200] as const
export type UiScale = typeof UI_SCALE_OPTIONS[number]

interface StoredPrefs {
  uiScale?: number
}

interface UiPreferencesState {
  uiScale: UiScale
  setUiScale: (scale: UiScale) => void
}

function hydrate(): UiScale {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return 100
    const parsed = JSON.parse(raw) as StoredPrefs
    if (parsed.uiScale && (UI_SCALE_OPTIONS as readonly number[]).includes(parsed.uiScale)) {
      return parsed.uiScale as UiScale
    }
  } catch {
    /* ignore */
  }
  return 100
}

function persist(prefs: StoredPrefs) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs))
  } catch {
    /* ignore */
  }
}

function applyScale(scale: UiScale) {
  const root = document.documentElement as HTMLElement & { style: CSSStyleDeclaration & { zoom?: string } }
  root.style.zoom = String(scale / 100)
}

export const useUiPreferencesStore = create<UiPreferencesState>((set, get) => ({
  uiScale: hydrate(),

  setUiScale: (scale) => {
    if (scale === get().uiScale) return
    persist({ uiScale: scale })
    applyScale(scale)
    set({ uiScale: scale })
  },
}))

if (typeof window !== 'undefined') {
  applyScale(hydrate())
}
