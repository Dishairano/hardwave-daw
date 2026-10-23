import { create } from 'zustand'

const LS_ENABLED = 'hardwave.daw.metronomeEnabled'
const LS_VOLUME = 'hardwave.daw.metronomeVolume'
const LS_ACCENT = 'hardwave.daw.metronomeAccent'
const LS_REC_ONLY = 'hardwave.daw.metronomeRecordOnly'
const LS_PRECOUNT = 'hardwave.daw.metronomePrecountBars'
// Paths to the user's own click sounds. These replace data-URL keys that no
// screen ever wrote.
const LS_BEAT_SOUND = 'hardwave.daw.metronomeBeatSound'
const LS_DOWNBEAT_SOUND = 'hardwave.daw.metronomeDownbeatSound'

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
  /** A file of the user's own for every beat; null for the built-in click. */
  beatSound: string | null
  /** A file for the bar's first beat; null to use the beat sound. */
  downbeatSound: string | null

  setEnabled: (v: boolean) => void
  toggleEnabled: () => void
  setVolume: (v: number) => void
  setAccent: (v: boolean) => void
  setRecordOnly: (v: boolean) => void
  setPrecountBars: (v: number) => void
  setBeatSound: (path: string | null) => void
  setDownbeatSound: (path: string | null) => void
}

export const useMetronomeStore = create<MetronomeState>((set) => ({
  enabled: readBool(LS_ENABLED, false),
  volume: Math.max(0, Math.min(1, readNumber(LS_VOLUME, 0.5))),
  accent: readBool(LS_ACCENT, true),
  recordOnly: readBool(LS_REC_ONLY, false),
  precountBars: Math.max(0, Math.min(4, Math.round(readNumber(LS_PRECOUNT, 0)))),
  beatSound: readString(LS_BEAT_SOUND),
  downbeatSound: readString(LS_DOWNBEAT_SOUND),

  setEnabled: (v) => { writeBool(LS_ENABLED, v); set({ enabled: v }) },
  toggleEnabled: () => set(s => { writeBool(LS_ENABLED, !s.enabled); return { enabled: !s.enabled } }),
  setVolume: (v) => { const c = Math.max(0, Math.min(1, v)); writeNumber(LS_VOLUME, c); set({ volume: c }) },
  setAccent: (v) => { writeBool(LS_ACCENT, v); set({ accent: v }) },
  setRecordOnly: (v) => { writeBool(LS_REC_ONLY, v); set({ recordOnly: v }) },
  setPrecountBars: (v) => {
    const c = Math.max(0, Math.min(4, Math.round(v)))
    writeNumber(LS_PRECOUNT, c); set({ precountBars: c })
  },
  setBeatSound: (path) => { writeString(LS_BEAT_SOUND, path); set({ beatSound: path }) },
  setDownbeatSound: (path) => { writeString(LS_DOWNBEAT_SOUND, path); set({ downbeatSound: path }) },
}))
