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
  setMode: (mode: AutomationWriteMode) => void
  cycleMode: () => void
  /** True when a control movement should be recorded. */
  isRecording: () => boolean
  beginTouch: (trackId: string, target: AutomationTargetInfo) => void
  writeSample: (trackId: string, target: AutomationTargetInfo, normalized: number) => void
  endTouch: (trackId: string, target: AutomationTargetInfo) => void
}

const ORDER: AutomationWriteMode[] = ['off', 'write', 'touch', 'latch']

function keyOf(trackId: string, target: AutomationTargetInfo): string {
  return `${trackId}:${JSON.stringify(target)}`
}

export const useAutomationWriteStore = create<AutomationWriteState>((set, get) => ({
  mode: 'off',
  touching: [],

  setMode: (mode) => {
    set({ mode, touching: [] })
    invoke('set_automation_write_mode', { mode }).catch(() => {})
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
