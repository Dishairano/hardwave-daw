import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useMetronomeStore } from './metronomeStore'

export type SnapValue =
  | 'Off' | '1/1' | '1/2' | '1/4' | '1/8' | '1/16' | '1/32' | '1/64'
  | '1/4T' | '1/8T' | '1/16T' | '1/4D' | '1/8D'

export const SNAP_VALUES: SnapValue[] = [
  'Off', '1/1', '1/2', '1/4', '1/8', '1/16', '1/32', '1/64',
  '1/4T', '1/8T', '1/16T', '1/4D', '1/8D',
]

const PPQ_TICKS = 960

// ── Pre-count metronome ──────────────────────────────────────────────────
let precountCtx: AudioContext | null = null
let precountCancel: (() => void) | null = null
let precountState: { beat: number; total: number } | null = null
const precountListeners = new Set<(s: typeof precountState) => void>()

export function subscribePrecount(cb: (s: typeof precountState) => void) {
  precountListeners.add(cb)
  cb(precountState)
  return () => { precountListeners.delete(cb) }
}

function notifyPrecount() {
  for (const cb of precountListeners) cb(precountState)
}

function cancelPrecount() {
  if (precountCancel) { precountCancel(); precountCancel = null }
  precountState = null
  notifyPrecount()
}

function runPrecountClicks(totalBeats: number, bpb: number, beatSec: number, volume: number, accent: boolean): Promise<void> {
  return new Promise<void>((resolve) => {
    cancelPrecount()
    const AC: typeof AudioContext | undefined = (window as unknown as { AudioContext?: typeof AudioContext; webkitAudioContext?: typeof AudioContext }).AudioContext
      || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
    if (!AC) { resolve(); return }
    const ctx = precountCtx ?? new AC()
    precountCtx = ctx
    if (ctx.state === 'suspended') ctx.resume().catch(() => {})
    const startAt = ctx.currentTime + 0.05
    for (let i = 0; i < totalBeats; i++) {
      const t = startAt + i * beatSec
      const osc = ctx.createOscillator()
      const gain = ctx.createGain()
      osc.type = 'sine'
      osc.frequency.value = (accent && i % bpb === 0) ? 1500 : 800
      const peak = Math.max(0.0001, volume) * 0.6
      gain.gain.setValueAtTime(0, t)
      gain.gain.linearRampToValueAtTime(peak, t + 0.002)
      gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.055)
      osc.connect(gain).connect(ctx.destination)
      osc.start(t); osc.stop(t + 0.08)
    }
    precountState = { beat: 0, total: totalBeats }
    notifyPrecount()
    const tickInterval = setInterval(() => {
      if (!precountState) return
      const elapsed = ctx.currentTime - startAt
      const b = Math.max(0, Math.min(totalBeats, Math.floor(elapsed / beatSec)))
      if (precountState.beat !== b) {
        precountState = { beat: b, total: totalBeats }
        notifyPrecount()
      }
    }, 30)
    const done = setTimeout(() => {
      clearInterval(tickInterval)
      precountState = null
      precountCancel = null
      notifyPrecount()
      resolve()
    }, totalBeats * beatSec * 1000 + 80)
    precountCancel = () => {
      clearTimeout(done)
      clearInterval(tickInterval)
      resolve()
    }
  })
}
// Returns tick count for a given snap value, or 0 when snap is disabled.
export function snapToTicks(snap: SnapValue, enabled: boolean): number {
  if (!enabled || snap === 'Off') return 0
  const base: Record<string, number> = {
    '1/1': PPQ_TICKS * 4,
    '1/2': PPQ_TICKS * 2,
    '1/4': PPQ_TICKS,
    '1/8': PPQ_TICKS / 2,
    '1/16': PPQ_TICKS / 4,
    '1/32': PPQ_TICKS / 8,
    '1/64': PPQ_TICKS / 16,
  }
  if (snap.endsWith('T')) {
    const root = snap.slice(0, -1)
    return Math.round((base[root] || PPQ_TICKS) * 2 / 3)
  }
  if (snap.endsWith('D')) {
    const root = snap.slice(0, -1)
    return Math.round((base[root] || PPQ_TICKS) * 3 / 2)
  }
  return base[snap] || PPQ_TICKS
}

interface TransportState {
  playing: boolean
  recording: boolean
  looping: boolean
  positionSamples: number
  bpm: number
  sampleRate: number
  loopStart: number
  loopEnd: number
  masterVolumeDb: number
  timeSigNumerator: number
  timeSigDenominator: number
  patternMode: boolean
  trackHeight: number
  snapValue: SnapValue
  snapEnabled: boolean
  horizontalZoom: number
  clipColorOverrides: Record<string, string>
  editCursorTicks: number | null
  punchEnabled: boolean
  punchInTicks: number | null
  punchOutTicks: number | null

  play: () => void
  stop: () => void
  togglePlayback: () => void
  setPosition: (pos: number) => void
  setBpm: (bpm: number) => void
  toggleLoop: () => void
  setLoop: (start: number, end: number) => void
  setMasterVolume: (db: number) => void
  setTimeSignature: (num: number, den: number) => void
  setPatternMode: (enabled: boolean) => void
  setTrackHeight: (height: number) => void
  setSnapValue: (v: SnapValue) => void
  toggleSnap: () => void
  setHorizontalZoom: (z: number) => void
  zoomToFit: () => void
  setClipColor: (clipId: string, color: string | null) => void
  setEditCursor: (ticks: number | null) => void
  togglePunch: () => void
  setPunchIn: (ticks: number | null) => void
  setPunchOut: (ticks: number | null) => void
  clearPunch: () => void
  setPunchRangeFromLoop: () => void
  tapTempo: () => void
  startListening: () => void
}

const TAP_WINDOW_MS = 2000
const tapTimes: number[] = []

const LS_PUNCH_ENABLED = 'hardwave.daw.punchEnabled'
const LS_PUNCH_IN = 'hardwave.daw.punchIn'
const LS_PUNCH_OUT = 'hardwave.daw.punchOut'
function readTicks(key: string): number | null {
  try {
    const v = localStorage.getItem(key)
    if (v == null || v === '') return null
    const n = parseInt(v, 10)
    return Number.isFinite(n) && n >= 0 ? n : null
  } catch { return null }
}
function writeTicks(key: string, v: number | null) {
  try {
    if (v == null) localStorage.removeItem(key); else localStorage.setItem(key, String(v))
  } catch {}
}
function readBool(key: string, fallback: boolean): boolean {
  try {
    const raw = localStorage.getItem(key)
    if (raw == null) return fallback
    return raw === '1' || raw === 'true'
  } catch { return fallback }
}
function writeBool(key: string, v: boolean) {
  try { localStorage.setItem(key, v ? '1' : '0') } catch {}
}

export const useTransportStore = create<TransportState>((set, get) => ({
  playing: false,
  recording: false,
  looping: false,
  positionSamples: 0,
  bpm: 140,
  sampleRate: 48000,
  loopStart: 0,
  loopEnd: 0,
  masterVolumeDb: 0,
  timeSigNumerator: 4,
  timeSigDenominator: 4,
  patternMode: false,
  trackHeight: 56,
  snapValue: '1/4',
  snapEnabled: true,
  horizontalZoom: 1,
  clipColorOverrides: {},
  editCursorTicks: null,
  punchEnabled: readBool(LS_PUNCH_ENABLED, false),
  punchInTicks: readTicks(LS_PUNCH_IN),
  punchOutTicks: readTicks(LS_PUNCH_OUT),

  play: () => {
    const m = useMetronomeStore.getState()
    if (m.precountBars > 0 && m.enabled) {
      const { bpm, timeSigNumerator } = get()
      const bpb = timeSigNumerator > 0 ? timeSigNumerator : 4
      const totalBeats = m.precountBars * bpb
      const beatSec = 60 / Math.max(1, bpm)
      runPrecountClicks(totalBeats, bpb, beatSec, m.volume, m.accent)
        .then(() => {
          if (get().playing) return
          invoke('play'); set({ playing: true })
        })
      set({ playing: false })
      return
    }
    invoke('play'); set({ playing: true })
  },
  stop: () => { cancelPrecount(); invoke('stop') },
  togglePlayback: () => {
    if (get().playing) { get().stop() } else { get().play() }
  },
  setPosition: (pos) => invoke('set_position', { position: pos }),
  setBpm: (bpm) => invoke('set_bpm', { bpm }),
  toggleLoop: () => {
    invoke('toggle_loop')
    set(s => ({ looping: !s.looping }))
  },
  setLoop: (start, end) => {
    invoke('set_loop', { start, end })
    set({ loopStart: start, loopEnd: end })
  },
  setMasterVolume: (db) => {
    invoke('set_master_volume', { db })
    set({ masterVolumeDb: db })
  },
  setTimeSignature: (num, den) => {
    invoke('set_time_signature', { numerator: num, denominator: den })
    set({ timeSigNumerator: num, timeSigDenominator: den })
  },
  setPatternMode: (enabled) => {
    invoke('set_pattern_mode', { enabled })
    set({ patternMode: enabled })
  },
  setTrackHeight: (height) => set({ trackHeight: Math.min(200, Math.max(24, height)) }),
  setSnapValue: (v) => set({ snapValue: v, snapEnabled: v !== 'Off' ? true : false }),
  toggleSnap: () => set(s => ({ snapEnabled: !s.snapEnabled })),
  setHorizontalZoom: (z) => set({ horizontalZoom: Math.max(0.1, Math.min(16, z)) }),
  zoomToFit: () => set({ horizontalZoom: 1 }),
  setEditCursor: (ticks) => set({ editCursorTicks: ticks == null ? null : Math.max(0, Math.floor(ticks)) }),
  setClipColor: (clipId, color) => set(s => {
    const next = { ...s.clipColorOverrides }
    if (color == null) delete next[clipId]; else next[clipId] = color
    return { clipColorOverrides: next }
  }),
  togglePunch: () => {
    const next = !get().punchEnabled
    writeBool(LS_PUNCH_ENABLED, next)
    set({ punchEnabled: next })
  },
  setPunchIn: (ticks) => {
    const v = ticks == null ? null : Math.max(0, Math.floor(ticks))
    writeTicks(LS_PUNCH_IN, v)
    set({ punchInTicks: v })
  },
  setPunchOut: (ticks) => {
    const v = ticks == null ? null : Math.max(0, Math.floor(ticks))
    writeTicks(LS_PUNCH_OUT, v)
    set({ punchOutTicks: v })
  },
  clearPunch: () => {
    writeTicks(LS_PUNCH_IN, null)
    writeTicks(LS_PUNCH_OUT, null)
    set({ punchInTicks: null, punchOutTicks: null })
  },
  setPunchRangeFromLoop: () => {
    const { loopStart, loopEnd, sampleRate, bpm } = get()
    if (!(loopEnd > loopStart) || sampleRate <= 0 || bpm <= 0) return
    const inTicks = Math.max(0, Math.round((loopStart / sampleRate) * (bpm / 60) * PPQ_TICKS))
    const outTicks = Math.max(inTicks + 1, Math.round((loopEnd / sampleRate) * (bpm / 60) * PPQ_TICKS))
    writeTicks(LS_PUNCH_IN, inTicks)
    writeTicks(LS_PUNCH_OUT, outTicks)
    writeBool(LS_PUNCH_ENABLED, true)
    set({ punchInTicks: inTicks, punchOutTicks: outTicks, punchEnabled: true })
  },
  tapTempo: () => {
    const now = Date.now()
    if (tapTimes.length > 0 && now - tapTimes[tapTimes.length - 1] > TAP_WINDOW_MS) {
      tapTimes.length = 0
    }
    tapTimes.push(now)
    if (tapTimes.length >= 2) {
      const intervals = []
      for (let i = 1; i < tapTimes.length; i++) {
        intervals.push(tapTimes[i] - tapTimes[i - 1])
      }
      const avg = intervals.reduce((a, b) => a + b, 0) / intervals.length
      const bpm = Math.round(60000 / avg)
      if (bpm >= 20 && bpm <= 999) {
        get().setBpm(bpm)
      }
    }
    if (tapTimes.length > 8) tapTimes.shift()
  },

  startListening: () => {
    listen<{
      position: number
      playing: boolean
      bpm: number
      masterVolumeDb: number
      timeSig: [number, number]
      patternMode: boolean
      looping: boolean
      loopStart: number
      loopEnd: number
    }>('daw:transport', (event) => {
      set({
        positionSamples: event.payload.position,
        playing: event.payload.playing,
        bpm: event.payload.bpm,
        masterVolumeDb: event.payload.masterVolumeDb,
        timeSigNumerator: event.payload.timeSig[0],
        timeSigDenominator: event.payload.timeSig[1],
        patternMode: event.payload.patternMode,
        looping: event.payload.looping,
        loopStart: event.payload.loopStart,
        loopEnd: event.payload.loopEnd,
      })
    })
  },
}))
