import { create } from 'zustand'

const LS_ENABLED = 'hardwave.daw.metronomeEnabled'
const LS_VOLUME = 'hardwave.daw.metronomeVolume'
const LS_ACCENT = 'hardwave.daw.metronomeAccent'
const LS_REC_ONLY = 'hardwave.daw.metronomeRecordOnly'
const LS_PRECOUNT = 'hardwave.daw.metronomePrecountBars'
const LS_CUSTOM_DOWN = 'hardwave.daw.metronomeCustomDownbeat'
const LS_CUSTOM_ACCENT = 'hardwave.daw.metronomeCustomAccent'
const LS_CUSTOM_DOWN_NAME = 'hardwave.daw.metronomeCustomDownbeatName'
const LS_CUSTOM_ACCENT_NAME = 'hardwave.daw.metronomeCustomAccentName'

function readString(key: string): string | null {
  try {
    const v = localStorage.getItem(key)
    return v && v.length > 0 ? v : null
  } catch { return null }
}
function writeString(key: string, v: string | null) {
  try {
    if (v == null) localStorage.removeItem(key)
    else localStorage.setItem(key, v)
  } catch {}
}

function readNumber(key: string, fallback: number): number {
  try {
    const v = parseFloat(localStorage.getItem(key) || '')
    return Number.isFinite(v) ? v : fallback
  } catch { return fallback }
}
function readBool(key: string, fallback: boolean): boolean {
  try {
    const raw = localStorage.getItem(key)
    if (raw === null) return fallback
    return raw === '1' || raw === 'true'
  } catch { return fallback }
}
function writeBool(key: string, v: boolean) { try { localStorage.setItem(key, v ? '1' : '0') } catch {} }
function writeNumber(key: string, v: number) { try { localStorage.setItem(key, String(v)) } catch {} }

interface MetronomeState {
  enabled: boolean
  volume: number
  accent: boolean
  recordOnly: boolean
  precountBars: number
  customDownbeat: string | null
  customAccent: string | null
  customDownbeatName: string | null
  customAccentName: string | null

  setEnabled: (v: boolean) => void
  toggleEnabled: () => void
  setVolume: (v: number) => void
  setAccent: (v: boolean) => void
  setRecordOnly: (v: boolean) => void
  setPrecountBars: (v: number) => void
  setCustomDownbeat: (dataUrl: string | null, name: string | null) => void
  setCustomAccent: (dataUrl: string | null, name: string | null) => void
}

export const useMetronomeStore = create<MetronomeState>((set) => ({
  enabled: readBool(LS_ENABLED, false),
  volume: Math.max(0, Math.min(1, readNumber(LS_VOLUME, 0.5))),
  accent: readBool(LS_ACCENT, true),
  recordOnly: readBool(LS_REC_ONLY, false),
  precountBars: Math.max(0, Math.min(4, Math.round(readNumber(LS_PRECOUNT, 0)))),
  customDownbeat: readString(LS_CUSTOM_DOWN),
  customAccent: readString(LS_CUSTOM_ACCENT),
  customDownbeatName: readString(LS_CUSTOM_DOWN_NAME),
  customAccentName: readString(LS_CUSTOM_ACCENT_NAME),

  setEnabled: (v) => { writeBool(LS_ENABLED, v); set({ enabled: v }) },
  toggleEnabled: () => set(s => { writeBool(LS_ENABLED, !s.enabled); return { enabled: !s.enabled } }),
  setVolume: (v) => { const c = Math.max(0, Math.min(1, v)); writeNumber(LS_VOLUME, c); set({ volume: c }) },
  setAccent: (v) => { writeBool(LS_ACCENT, v); set({ accent: v }) },
  setRecordOnly: (v) => { writeBool(LS_REC_ONLY, v); set({ recordOnly: v }) },
  setPrecountBars: (v) => {
    const c = Math.max(0, Math.min(4, Math.round(v)))
    writeNumber(LS_PRECOUNT, c); set({ precountBars: c })
  },
  setCustomDownbeat: (dataUrl, name) => {
    writeString(LS_CUSTOM_DOWN, dataUrl)
    writeString(LS_CUSTOM_DOWN_NAME, name)
    set({ customDownbeat: dataUrl, customDownbeatName: name })
  },
  setCustomAccent: (dataUrl, name) => {
    writeString(LS_CUSTOM_ACCENT, dataUrl)
    writeString(LS_CUSTOM_ACCENT_NAME, name)
    set({ customAccent: dataUrl, customAccentName: name })
  },
}))
