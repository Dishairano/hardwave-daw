import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export type SnapValue =
  | 'Off' | '1/1' | '1/2' | '1/4' | '1/8' | '1/16' | '1/32' | '1/64'
  | '1/4T' | '1/8T' | '1/16T' | '1/4D' | '1/8D'

export const SNAP_VALUES: SnapValue[] = [
  'Off', '1/1', '1/2', '1/4', '1/8', '1/16', '1/32', '1/64',
  '1/4T', '1/8T', '1/16T', '1/4D', '1/8D',
]

const PPQ_TICKS = 960
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
  tapTempo: () => void
  startListening: () => void
}

const TAP_WINDOW_MS = 2000
const tapTimes: number[] = []

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

  play: () => { invoke('play'); set({ playing: true }) },
  stop: () => { invoke('stop') },
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
  setClipColor: (clipId, color) => set(s => {
    const next = { ...s.clipColorOverrides }
    if (color == null) delete next[clipId]; else next[clipId] = color
    return { clipColorOverrides: next }
  }),
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
