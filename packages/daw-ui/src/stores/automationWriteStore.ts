import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { AutomationTargetInfo } from './trackStore'

/**
 * Automation write mode, and the plumbing that records a control movement
 * into a lane.
 *
 * The recorder in `hardwave_project::automation_recording` has existed with
 * tests since the automation work and had no caller: `push_sample` was never
 * called outside the engine's own test, so moving a fader while the transport
 * ran wrote no automation. These are the calls that make the feature real.
 *
 * Deliberately not persisted. Reopening the DAW in Write mode would overwrite
 * a lane the moment a fader moved during playback, so every session starts in
 * Off and the producer chooses.
 */
export type AutomationWriteMode = 'off' | 'write' | 'touch' | 'latch'

/** Fader range the engine maps a volume lane onto: -60 dB to +6 dB. */
export const VOLUME_LANE_MIN_DB = -60
export const VOLUME_LANE_MAX_DB = 6

export function normalizeVolumeDb(db: number): number {
  const span = VOLUME_LANE_MAX_DB - VOLUME_LANE_MIN_DB
  return Math.min(1, Math.max(0, (db - VOLUME_LANE_MIN_DB) / span))
}

/** Pan is stored -1..1 on the control and 0..1 in the lane. */
export function normalizePan(pan: number): number {
  return Math.min(1, Math.max(0, (pan + 1) / 2))
}

interface AutomationWriteState {
  mode: AutomationWriteMode
  /** Targets with a gesture in progress, so a drag opens one session. */
  touching: string[]
  /** Ride the curve that is already there rather than replacing it. */
  trim: boolean
  setMode: (mode: AutomationWriteMode) => void
  setTrim: (trim: boolean) => void
  cycleMode: () => void
  /** True when a control movement should be recorded. */
  isRecording: () => boolean
  beginTouch: (trackId: string, target: AutomationTargetInfo) => void
  writeSample: (trackId: string, target: AutomationTargetInfo, normalized: number) => void
  endTouch: (trackId: string, target: AutomationTargetInfo) => void
  /**
   * A control without a press and release of its own (a knob in a plug-in
   * window, a value typed in) moved. Records it like a drag; the gesture
   * ends once the control has been still for a moment.
   */
  writeFromControl: (trackId: string, target: AutomationTargetInfo, normalized: number) => void
}

/** How long a control must be still before its gesture counts as let go. */
const IDLE_END_MS = 400
const idleTimers = new Map<string, ReturnType<typeof setTimeout>>()

const ORDER: AutomationWriteMode[] = ['off', 'write', 'touch', 'latch']

function keyOf(trackId: string, target: AutomationTargetInfo): string {
  return `${trackId}:${JSON.stringify(target)}`
}

/**
 * Follow the knobs inside plug-in windows while automation is being
 * written.
 *
 * The app moves the faders and the generic parameter sheet, so those
 * recorded from the start. A knob in a plug-in's own window is moved by
 * the plug-in: the host keeps a log of those moves and this drains it
 * on a timer and writes each one like any other control.
 */
// Plain setInterval rather than window.setInterval: the store is also
// loaded in tests, which run without a window.
let knobPoll: ReturnType<typeof setInterval> | null = null
// A drain still waiting for its answer: the next tick skips rather than
// piling another request behind it while the app's main thread is busy.
let draining = false

function startKnobPolling() {
  if (knobPoll !== null) return
  knobPoll = setInterval(() => {
    const store = useAutomationWriteStore.getState()
    // The mode can be put back to off without going through setMode, so
    // the tick checks rather than trusting that it was stopped.
    if (!store.isRecording()) {
      stopKnobPolling()
      return
    }
    if (draining) return
    draining = true
    Promise.all([
      invoke<{ trackId: string; slotId: string; paramId: number; value: number }[]>('drain_plugin_knob_moves')
        .catch(() => []),
      invoke<MappedMove[]>('drain_midi_mapped_moves').catch(() => []),
    ])
      .finally(() => { draining = false })
      .then(([knobs, mapped]) => {
        if (!store.isRecording()) return
        for (const move of Array.isArray(knobs) ? knobs : []) {
          // Through writeFromControl, so the pass ends when the knob stops:
          // these moves opened a session that nothing ever closed, so they
          // were never written into a lane.
          store.writeFromControl(
            move.trackId,
            { kind: 'plugin_param', slotId: move.slotId, paramId: move.paramId },
            move.value,
          )
        }
        // Controls a learned MIDI controller moved: the fader or knob moved
        // and nothing wrote it into a lane.
        for (const move of Array.isArray(mapped) ? mapped : []) {
          const target = mappedTarget(move)
          if (target) store.writeFromControl(move.trackId, target, move.value)
        }
      })
      .catch(() => { /* no engine: nothing to follow */ })
  }, 60)
}

/** A control a mapped MIDI controller moved (`drain_midi_mapped_moves`). */
interface MappedMove {
  trackId: string
  kind: 'track_volume' | 'track_pan' | 'track_mute' | 'plugin_param'
  slotId?: string
  paramId?: number
  value: number
}

export function mappedTarget(move: MappedMove): AutomationTargetInfo | null {
  switch (move.kind) {
    case 'track_volume':
    case 'track_pan':
    case 'track_mute':
      return { kind: move.kind }
    case 'plugin_param':
      return move.slotId != null && move.paramId != null
        ? { kind: 'plugin_param', slotId: move.slotId, paramId: move.paramId }
        : null
    default:
      return null
  }
}

function stopKnobPolling() {
  if (knobPoll === null) return
  clearInterval(knobPoll)
  knobPoll = null
}

export const useAutomationWriteStore = create<AutomationWriteState>((set, get) => ({
  mode: 'off',
  trim: false,
  touching: [],

  setMode: (mode) => {
    set({ mode, touching: [] })
    invoke('set_automation_write_mode', { mode }).catch(() => {})
    // Only poll while something is being recorded: an idle DAW has no
    // reason to ask the engine anything sixteen times a second.
    if (mode === 'off') {
      stopKnobPolling()
      invoke('drain_plugin_knob_moves').catch(() => {})
      invoke('drain_midi_mapped_moves').catch(() => {})
    } else {
      // Moves made while nothing was recording are not part of this pass.
      Promise.all([
        invoke('drain_plugin_knob_moves').catch(() => {}),
        invoke('drain_midi_mapped_moves').catch(() => {}),
      ]).finally(startKnobPolling)
    }
  },

  setTrim: (trim) => {
    set({ trim })
    invoke('set_automation_trim', { trim }).catch(() => {})
  },

  cycleMode: () => {
    const next = ORDER[(ORDER.indexOf(get().mode) + 1) % ORDER.length]
    get().setMode(next)
  },

  isRecording: () => get().mode !== 'off',

  beginTouch: (trackId, target) => {
    if (!get().isRecording()) return
    const key = keyOf(trackId, target)
    if (get().touching.includes(key)) return
    set((s) => ({ touching: [...s.touching, key] }))
    invoke('automation_touch_begin', { trackId, target }).catch(() => {})
  },

  writeSample: (trackId, target, normalized) => {
    if (!get().isRecording()) return
    // A drag that never opened a session still records: the first sample
    // opens one, so a component only has to stream values.
    get().beginTouch(trackId, target)
    invoke('automation_write_sample', { trackId, target, value: normalized }).catch(() => {})
  },

  writeFromControl: (trackId, target, normalized) => {
    if (!get().isRecording()) return
    get().writeSample(trackId, target, normalized)
    const key = keyOf(trackId, target)
    const prev = idleTimers.get(key)
    if (prev) clearTimeout(prev)
    idleTimers.set(key, setTimeout(() => {
      idleTimers.delete(key)
      get().endTouch(trackId, target)
    }, IDLE_END_MS))
  },

  endTouch: (trackId, target) => {
    const key = keyOf(trackId, target)
    if (!get().touching.includes(key)) return
    set((s) => ({ touching: s.touching.filter((k) => k !== key) }))
    invoke('automation_touch_end', { trackId, target })
      .then(async (laneId) => {
        if (!laneId) return
        // The lane the pass wrote into: refresh so it is on screen.
        const { useTrackStore } = await import('./trackStore')
        await useTrackStore.getState().fetchTracks()
      })
      .catch(() => {})
  },
}))
