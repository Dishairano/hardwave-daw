import { create } from 'zustand'

const STORAGE_KEY = 'hardwave.daw.mixerSettings'

interface StoredSettings {
  useNewMixer?: boolean
  showWidthKnob?: boolean
}

interface MixerSettingsState {
  useNewMixer: boolean
  showWidthKnob: boolean
  setUseNewMixer: (on: boolean) => void
  setShowWidthKnob: (on: boolean) => void
}

function hydrate(): StoredSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw) return JSON.parse(raw) as StoredSettings
  } catch {
    /* ignore */
  }
  return {}
}

function persist(next: StoredSettings) {
  try {
    const existing = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}')
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...existing, ...next }))
  } catch {
    /* ignore */
  }
}

const initial = hydrate()

export const useMixerSettingsStore = create<MixerSettingsState>((set) => ({
  useNewMixer: initial.useNewMixer ?? false,
  showWidthKnob: initial.showWidthKnob ?? true,
  setUseNewMixer: (on) => {
    persist({ useNewMixer: on })
    set({ useNewMixer: on })
  },
  setShowWidthKnob: (on) => {
    persist({ showWidthKnob: on })
    set({ showWidthKnob: on })
  },
}))
