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

// ── Count-in ─────────────────────────────────────────────────────────────
//
// The engine counts off and starts playback itself. This used to schedule a
// row of WebAudio oscillators and call play from a setTimeout, so the clicks
// came from a different clock than the audio and playback began wherever the
// timeout landed. What is left here is the on-screen counter, which polls the
// engine for how far the count has got.

let countInState: { beat: number; total: number } | null = null
const countInListeners = new Set<(s: typeof countInState) => void>()
let countInPoll: ReturnType<typeof setInterval> | null = null

export function subscribePrecount(cb: (s: typeof countInState) => void) {
  countInListeners.add(cb)
  cb(countInState)
  return () => { countInListeners.delete(cb) }
}

function notifyCountIn() {
  for (const cb of countInListeners) cb(countInState)
}

function stopCountInPolling() {
  if (countInPoll) { clearInterval(countInPoll); countInPoll = null }
  if (countInState !== null) {
    countInState = null
    notifyCountIn()
  }
}

/** Follow the engine's count-in so the counter on screen matches the clicks. */
function watchCountIn() {
  if (countInPoll) return
  countInPoll = setInterval(() => {
    invoke<{ active: boolean; beat: number; totalBeats: number }>('get_count_in_state')
      .then(({ active, beat, totalBeats }) => {
        if (!active) {
          stopCountInPolling()
          // The engine starts playback itself at the end of the count.
          useTransportStore.setState({ playing: true })
          return
        }
        if (!countInState || countInState.beat !== beat) {
          countInState = { beat, total: totalBeats }
          notifyCountIn()
        }
      })
      .catch(() => stopCountInPolling())
  }, 40)
}

function cancelPrecount() {
  stopCountInPolling()
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
  /** Sample position the playhead was at when `recording` flipped on.
   * Used to commit_recording_to_midi_clip with the right window on the
   * trailing edge of toggleRecording. `null` when not recording. */
  recordStartSample: number | null
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
  toggleRecording: () => void
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

// The punch range is a position in the song, so it is saved in the project's
// timeline_state (see stores/timelineState.ts). It used to sit in
// localStorage, which meant it stayed behind on one machine and leaked into
// whatever project you opened next.

export const useTransportStore = create<TransportState>((set, get) => ({
  playing: false,
  recording: false,
  recordStartSample: null,
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
  punchEnabled: false,
  punchInTicks: null,
  punchOutTicks: null,

  play: () => {
    const m = useMetronomeStore.getState()
    if (m.precountBars > 0 && m.enabled) {
      // The engine counts off and starts playback on the sample the count
      // ends; this only follows along for the counter on screen.
      invoke('start_count_in', { bars: m.precountBars }).catch(() => {
        // If the command is missing, start rather than stranding the user.
        invoke('play'); set({ playing: true })
      })
      set({ playing: false })
      watchCountIn()
      return
    }
    invoke('play'); set({ playing: true })
  },
  stop: async () => {
    cancelPrecount()
    // If a recording is in flight when Stop fires (typically Spacebar
    // mid-record), the Rust side finalises the capture buffer and
    // returns the WAV path so we can drop the take on the first armed
    // track — same trailing-edge behaviour as toggleRecording. Without
    // this, Space mid-record silently discarded the take.
    const wasRecording = get().recording
    if (wasRecording) set({ recording: false })
    try {
      const path = (await invoke('stop')) as string | null
      if (wasRecording && path) {
        const { useTrackStore } = await import('./trackStore')
        const armedTrack = useTrackStore.getState().tracks.find(t => t.armed)
        if (armedTrack) {
          await useTrackStore.getState().importAudioFile(armedTrack.id, path, 0)
        }
      }
    } catch (e) {
      if (wasRecording) set({ recording: wasRecording })
      throw e
    }
  },
  togglePlayback: () => {
    if (get().playing) { get().stop() } else { get().play() }
  },
  setPosition: (pos) => invoke('set_position', { position: pos }),
  setBpm: (bpm) => invoke('set_bpm', { bpm }),
  toggleLoop: () => {
    invoke('toggle_loop')
    set(s => ({ looping: !s.looping }))
  },
  toggleRecording: async () => {
    // Engine flips the recording flag and starts/stops the capture
    // session. On the trailing edge it returns the path of the WAV
    // that was just written to disk; we then drop a clip on the first
    // armed track so the take is immediately visible on the timeline.
    //
    // Audio vs MIDI armed track branch:
    //   - Audio track: import the returned WAV path as an audio clip.
    //   - MIDI track:  drop the WAV path, invoke `commit_recording_to_midi_clip`
    //     to drain the engine's rolling capture ring into a MidiClip
    //     placed at the start position. The capture ring is always
    //     recording (see Page 7 work), so any notes played between the
    //     leading + trailing edges of this toggle land in the clip.
    const wasRecording = get().recording
    const startSample = wasRecording
      ? (get().recordStartSample ?? 0)
      : get().positionSamples
    set(() => ({
      recording: !wasRecording,
      recordStartSample: wasRecording ? null : startSample,
    }))
    try {
      const path = (await invoke('toggle_recording')) as string | null
      if (!wasRecording) return // leading edge, nothing more to do

      const { useTrackStore } = await import('./trackStore')
      const armedTrack = useTrackStore.getState().tracks.find(t => t.armed)
      if (!armedTrack) return

      const endSample = get().positionSamples
      if (armedTrack.kind === 'Midi') {
        try {
          const { useRecordingPrefsStore } = await import('./recordingPrefsStore')
          await invoke('commit_recording_to_midi_clip', {
            trackId: armedTrack.id,
            startSample,
            endSample,
            quantizeTicks: null,
            // Blend-record (Ctrl+B): merge into the overlapping clip
            // instead of stacking a new one.
            blend: useRecordingPrefsStore.getState().blendRecord,
          })
          await useTrackStore.getState().fetchTracks()
        } catch (err) {
          // No notes captured / no MIDI input — leave the take blank
          // and surface in console for debugging.
          console.warn('commit_recording_to_midi_clip:', err)
        }
      } else if (path) {
        await useTrackStore.getState().importAudioFile(armedTrack.id, path, 0)
      }
    } catch (e) {
      // Roll back the optimistic toggle if the engine rejected the call.
      set({ recording: wasRecording, recordStartSample: null })
      throw e
    }
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
    set({ punchEnabled: !get().punchEnabled })
  },
  setPunchIn: (ticks) => {
    set({ punchInTicks: ticks == null ? null : Math.max(0, Math.floor(ticks)) })
  },
  setPunchOut: (ticks) => {
    set({ punchOutTicks: ticks == null ? null : Math.max(0, Math.floor(ticks)) })
  },
  clearPunch: () => {
    set({ punchInTicks: null, punchOutTicks: null })
  },
  setPunchRangeFromLoop: () => {
    const { loopStart, loopEnd, sampleRate, bpm } = get()
    if (!(loopEnd > loopStart) || sampleRate <= 0 || bpm <= 0) return
    const inTicks = Math.max(0, Math.round((loopStart / sampleRate) * (bpm / 60) * PPQ_TICKS))
    const outTicks = Math.max(inTicks + 1, Math.round((loopEnd / sampleRate) * (bpm / 60) * PPQ_TICKS))
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
