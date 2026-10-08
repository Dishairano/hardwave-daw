import { create } from 'zustand'
import { noteBackendEvent } from '../services/frameStats'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useMetronomeStore } from './metronomeStore'
import { useNotificationStore } from './notificationStore'

export type SnapValue =
  | 'Off' | '1/1' | '1/2' | '1/4' | '1/8' | '1/16' | '1/32' | '1/64'
  | '1/4T' | '1/8T' | '1/16T' | '1/4D' | '1/8D'

export const SNAP_VALUES: SnapValue[] = [
  'Off', '1/1', '1/2', '1/4', '1/8', '1/16', '1/32', '1/64',
  '1/4T', '1/8T', '1/16T', '1/4D', '1/8D',
]

const PPQ_TICKS = 960

/** Matches CAPTURE_HEADROOM_SECS in crates/hardwave-engine/src/engine.rs. */
const RECORD_LIMIT_MINUTES = 20

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
  /** Sample the transport stops at for Play selection, or null. */
  playRangeEnd: number | null
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
  playRange: (startSamples: number, endSamples: number) => void
  clearPlayRange: () => void
  setMasterVolume: (db: number) => void
  setTimeSignature: (num: number, den: number) => void
  setPatternMode: (enabled: boolean) => void
  setTrackHeight: (height: number) => void
  setSnapValue: (v: SnapValue) => void
  toggleSnap: () => void
  setHorizontalZoom: (z: number) => void
  /** Fit the whole song into the playlist's width. */
  zoomToFit: () => void
  /**
   * Playlist width in pixels and the tick the last clip ends on, both
   * published by the arrangement. Held here so a fit can be asked for from a
   * menu or a shortcut without the caller knowing either.
   */
  playlistViewportPx: number
  songEndTicks: number
  setPlaylistMetrics: (px: number, songEndTicks: number) => void
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

/** Pixels per second of song at zoom 1. The playlist scales from this. */
export const PIXELS_PER_SECOND_BASE = 100

const TAP_WINDOW_MS = 2000
const tapTimes: number[] = []

// The punch range is a position in the song, so it is saved in the project's
// timeline_state (see stores/timelineState.ts). It used to sit in
// localStorage, which meant it stayed behind on one machine and leaked into
// whatever project you opened next.

/**
 * Tell the engine the punch range.
 *
 * The range lived only in this store and in the drawing: recording captured
 * the whole pass whatever the ruler showed. The engine converts ticks through
 * the project's tempo map, so a song with tempo changes punches in the right
 * place.
 *
 * A range that is not usable (no points, or out at or before in) is sent as
 * disabled rather than as a window, so a half-set range cannot silently
 * record nothing.
 */
export function pushPunchToEngine(): void {
  const { punchEnabled, punchInTicks, punchOutTicks } = useTransportStore.getState()
  const usable =
    punchEnabled && punchInTicks != null && punchOutTicks != null && punchOutTicks > punchInTicks
  invoke('set_punch_range', {
    enabled: usable,
    inTicks: usable ? punchInTicks : 0,
    outTicks: usable ? punchOutTicks : 0,
  }).catch(() => {})
}

/** One loop pass of a take, written as its own file. */
interface TakePass {
  path: string
  startSamples: number
  startTicks: number
  seconds: number
  peak: number
}

/** What the backend says a finished take turned out to be. */
interface RecordedTake {
  path: string | null
  seconds: number
  peak: number
  truncated: boolean
  /** One per loop pass, in the order played. One when the loop never wrapped. */
  passes: TakePass[]
}

/**
 * Put a finished take on the armed track, at the position it was recorded.
 *
 * Both record paths used to call `importAudioFile(trackId, path, 0)`, so a
 * take recorded from bar 9 landed at bar 1 and had to be dragged back. The
 * start position was already being tracked for the MIDI path and simply was
 * not passed here.
 *
 * The three ways a take can disappoint were also silent: nothing captured,
 * captured silence from the wrong input, and a take that hit the reserved
 * recording length. Each one now says so.
 */
/**
 * Put a finished recording on every armed track.
 *
 * Recording used to look at one track: `tracks.find(t => t.armed)`. Arming a
 * synth and a microphone recorded whichever came first in the list and threw
 * the other away without a word, and pressing Stop (rather than Record) in the
 * middle of a MIDI take committed nothing at all. Both are handled here, in
 * one place, so the two ways of ending a recording behave the same.
 *
 * An interface with one stereo input gives every armed audio track the same
 * two channels. Choosing which input feeds which track needs multi-channel
 * routing, which the DAW does not have yet, so the notification says so
 * rather than leaving someone to wonder why two tracks sound identical.
 */
async function commitRecording(
  take: RecordedTake | null,
  startSample: number,
  endSample: number,
): Promise<void> {
  const [{ useTrackStore }, { useNotificationStore }] = await Promise.all([
    import('./trackStore'),
    import('./notificationStore'),
  ])
  const push = useNotificationStore.getState().push

  const armed = useTrackStore.getState().tracks.filter(t => t.armed)
  const audioArmed = armed.filter(t => t.kind === 'Audio')
  const midiArmed = armed.filter(t => t.kind === 'Midi')
  const passes = take?.passes ?? []
  const haveAudio = Boolean(take?.path) && passes.length > 0

  if (armed.length === 0) {
    if (haveAudio) push('warning', 'Take saved, but no track is armed', { detail: take!.path! })
    return
  }

  // Where each pass starts comes from the engine, through the tempo map and
  // with the punch window applied. A backend without positions falls back to
  // where record was pressed.
  const state = useTransportStore.getState()
  const samplesPerTick = (state.sampleRate || 48000) * 60 / (Math.max(1, state.bpm) * PPQ_TICKS)
  const fallbackTicks = samplesPerTick > 0 ? Math.max(0, Math.round(startSample / samplesPerTick)) : 0

  const [{ useRecordingPrefsStore }] = await Promise.all([import('./recordingPrefsStore')])
  const blend = useRecordingPrefsStore.getState().blendRecord

  const audioDone: string[] = []
  const midiDone: string[] = []
  const failures: string[] = []
  let passCount = 0

  // One gesture, one undo: every armed track and every loop pass comes back
  // together rather than one undo per clip.
  await useTrackStore.getState().beginHistoryGroup()
  try {
    for (const track of audioArmed) {
      if (!haveAudio) continue
      const placed: string[] = []
      for (const pass of passes) {
        const ticks = Number.isFinite(pass.startTicks) ? pass.startTicks : fallbackTicks
        try {
          const clip = await useTrackStore.getState().importAudioFile(track.id, pass.path, ticks)
          if (clip?.clip_id) placed.push(clip.clip_id)
        } catch (err) {
          failures.push(`${track.name}: ${String(err)}`)
        }
      }
      // Loop recording: the latest pass plays, earlier ones stay on the track
      // muted, so trying another take is an unmute rather than a redo.
      for (const clipId of placed.slice(0, -1)) {
        await useTrackStore.getState().setClipMuted(track.id, clipId, true)
      }
      if (placed.length > 0) {
        audioDone.push(track.name)
        passCount = Math.max(passCount, placed.length)
      }
    }

    for (const track of midiArmed) {
      try {
        const clipIds = (await invoke('commit_recording_to_midi_clip', {
          trackId: track.id,
          startSample,
          endSample,
          quantizeTicks: null,
          // Blend-record (Ctrl+B): merge into the overlapping clip instead of
          // stacking a new one.
          blend,
        })) as string[]
        for (const clipId of clipIds.slice(0, -1)) {
          await useTrackStore.getState().setClipMuted(track.id, clipId, true)
        }
        if (clipIds.length > 0) {
          midiDone.push(track.name)
          passCount = Math.max(passCount, clipIds.length)
        }
      } catch (err) {
        failures.push(`${track.name}: ${String(err)}`)
      }
    }
  } finally {
    const tracksDone = audioDone.length + midiDone.length
    await useTrackStore.getState().endHistoryGroup(
      passCount > 1
        ? `Record ${passCount} loop passes on ${tracksDone} track${tracksDone === 1 ? '' : 's'}`
        : tracksDone > 1
          ? `Record ${tracksDone} tracks`
          : 'Record take',
    )
  }
  await useTrackStore.getState().fetchTracks()

  if (audioDone.length === 0 && midiDone.length === 0) {
    push('warning', 'Nothing was recorded', {
      detail: failures.length > 0
        ? failures.join('\n')
        : 'Check that the track is armed, that an input device is selected in Audio settings, and that your controller is enabled in the setup wizard.',
      sticky: true,
    })
    return
  }

  if (failures.length > 0) {
    push('warning', 'Part of the take did not land', { detail: failures.join('\n'), sticky: true })
  }
  if (audioDone.length + midiDone.length > 1) {
    push('info', `Recorded ${audioDone.length + midiDone.length} tracks`, {
      detail: audioDone.length > 1
        ? 'Your interface has one stereo input, so every armed audio track got the same take.'
        : [...audioDone, ...midiDone].join(', '),
    })
  }
  if (passCount > 1) {
    push('info', `Recorded ${passCount} loop passes`, {
      detail: 'The last pass is playing. The earlier ones are on the same track, muted: unmute one to use it instead.',
    })
  }
  if (haveAudio && audioArmed.length > 0) {
    if (take!.peak < 0.0005) {
      push('warning', 'That take is silent', {
        detail: 'The recording captured no signal. Check the input device and its level in Audio settings.',
        sticky: true,
      })
    } else if (take!.truncated) {
      push('warning', 'The take hit the recording limit', {
        detail: `Recording stops at ${RECORD_LIMIT_MINUTES} minutes in one take. What was captured up to that point has been kept.`,
        sticky: true,
      })
    }
  }
}

export const useTransportStore = create<TransportState>((set, get) => ({
  playing: false,
  recording: false,
  playRangeEnd: null,
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
    // mid-record), the Rust side finalises the capture buffer and returns
    // the WAV path, and the take goes onto the armed tracks exactly as it
    // does when Record is pressed again. Stop used to place the audio only,
    // so ending a MIDI take with the spacebar recorded nothing.
    const wasRecording = get().recording
    // Read the playhead before stopping: it is the end of the MIDI window,
    // and the transport may move it as it stops.
    const endSample = get().positionSamples
    if (wasRecording) set({ recording: false })
    try {
      const take = (await invoke('stop')) as RecordedTake | null
      if (wasRecording) {
        await commitRecording(take, get().recordStartSample ?? 0, endSample)
        set({ recordStartSample: null })
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
    // The engine flips the recording flag and starts or stops the capture
    // session. On the trailing edge it returns the WAV it just wrote, and
    // `commitRecording` puts that, and the MIDI captured in the same window,
    // on every armed track. The capture ring is always recording, so notes
    // played between the two edges of this toggle land in the clip.
    const wasRecording = get().recording
    const startSample = wasRecording
      ? (get().recordStartSample ?? 0)
      : get().positionSamples
    set(() => ({
      recording: !wasRecording,
      recordStartSample: wasRecording ? null : startSample,
    }))
    try {
      const take = (await invoke('toggle_recording')) as RecordedTake | null
      if (!wasRecording) return // leading edge, nothing more to do
      await commitRecording(take, startSample, get().positionSamples)
    } catch (e) {
      // Roll back the optimistic toggle if the engine rejected the call.
      set({ recording: wasRecording, recordStartSample: null })
      throw e
    }
  },
  /**
   * Play from `start` to `end` and stop, without touching the loop.
   *
   * There was no way to hear one part on its own: only loop, which changes
   * a setting the song keeps, and plain play, which runs to the end. The
   * watcher lives in `App.tsx` beside the punch one, so the stop happens on
   * the same playhead events rather than a timer that drifts.
   */
  playRange: (startSamples, endSamples) => {
    if (endSamples <= startSamples) return
    set({ playRangeEnd: endSamples })
    invoke('set_position', { position: startSamples })
    invoke('play')
    set({ playing: true, positionSamples: startSamples })
  },

  clearPlayRange: () => set({ playRangeEnd: null }),

  setLoop: (start, end) => {
    invoke('set_loop', { start, end })
    set({ loopStart: start, loopEnd: end })
  },
  setMasterVolume: (db) => {
    invoke('set_master_volume', { db })
    set({ masterVolumeDb: db })
  },
  setTimeSignature: (num, den) => {
    const previousNum = get().timeSigNumerator
    const previousDen = get().timeSigDenominator
    set({ timeSigNumerator: num, timeSigDenominator: den })
    invoke('set_time_signature', { numerator: num, denominator: den }).then(() => {
      // The playlist's bars come from the tempo map, which this writes.
      import('./tempoMapStore').then(m => m.useTempoMapStore.getState().refresh())
    }).catch(err => {
      // The backend rejects a signature that cannot be counted, such as 4/3,
      // where the denominator is not a note length. The toolbar must not keep
      // showing a signature the project does not have.
      set({ timeSigNumerator: previousNum, timeSigDenominator: previousDen })
      useNotificationStore.getState().push('warning', 'That time signature cannot be used', {
        detail: String(err),
      })
    })
  },
  setPatternMode: (enabled) => {
    invoke('set_pattern_mode', { enabled })
    set({ patternMode: enabled })
  },
  setTrackHeight: (height) => set({ trackHeight: Math.min(200, Math.max(24, height)) }),
  setSnapValue: (v) => set({ snapValue: v, snapEnabled: v !== 'Off' ? true : false }),
  toggleSnap: () => set(s => ({ snapEnabled: !s.snapEnabled })),
  setHorizontalZoom: (z) => set({ horizontalZoom: Math.max(0.1, Math.min(16, z)) }),
  // Was `set({ horizontalZoom: 1 })`: a reset dressed as a fit, which is why
  // nothing ever called it. Fitting needs two facts, the song's length and
  // the width available, so the arrangement publishes its width and the song
  // length comes from the clips.
  zoomToFit: () => {
    const { bpm, playlistViewportPx, songEndTicks: endTicks } = get()
    const width = playlistViewportPx
    if (width <= 0 || bpm <= 0) return
    if (endTicks <= 0) {
      // Nothing to fit: leave the zoom alone rather than snapping it.
      return
    }
    const seconds = (endTicks / PPQ_TICKS) * (60 / bpm)
    if (seconds <= 0) return
    // A small margin so the last clip does not touch the right edge.
    const target = (width * 0.97) / (seconds * PIXELS_PER_SECOND_BASE)
    set({ horizontalZoom: Math.max(0.1, Math.min(16, target)) })
  },
  playlistViewportPx: 0,
  songEndTicks: 0,
  setPlaylistMetrics: (px, songEndTicks) => {
    // Called from a render path, so do nothing when nothing changed.
    const s = get()
    if (s.playlistViewportPx === px && s.songEndTicks === songEndTicks) return
    set({ playlistViewportPx: px, songEndTicks })
  },
  setEditCursor: (ticks) => set({ editCursorTicks: ticks == null ? null : Math.max(0, Math.floor(ticks)) }),
  setClipColor: (clipId, color) => set(s => {
    const next = { ...s.clipColorOverrides }
    if (color == null) delete next[clipId]; else next[clipId] = color
    return { clipColorOverrides: next }
  }),
  togglePunch: () => {
    set({ punchEnabled: !get().punchEnabled })
    pushPunchToEngine()
  },
  setPunchIn: (ticks) => {
    set({ punchInTicks: ticks == null ? null : Math.max(0, Math.floor(ticks)) })
    pushPunchToEngine()
  },
  setPunchOut: (ticks) => {
    set({ punchOutTicks: ticks == null ? null : Math.max(0, Math.floor(ticks)) })
    pushPunchToEngine()
  },
  clearPunch: () => {
    set({ punchInTicks: null, punchOutTicks: null })
    pushPunchToEngine()
  },
  setPunchRangeFromLoop: () => {
    const { loopStart, loopEnd, sampleRate, bpm } = get()
    if (!(loopEnd > loopStart) || sampleRate <= 0 || bpm <= 0) return
    const inTicks = Math.max(0, Math.round((loopStart / sampleRate) * (bpm / 60) * PPQ_TICKS))
    const outTicks = Math.max(inTicks + 1, Math.round((loopEnd / sampleRate) * (bpm / 60) * PPQ_TICKS))
    set({ punchInTicks: inTicks, punchOutTicks: outTicks, punchEnabled: true })
    pushPunchToEngine()
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
      noteBackendEvent()
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
