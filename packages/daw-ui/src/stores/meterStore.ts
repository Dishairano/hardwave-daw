import { create } from 'zustand'
import { listen } from '@tauri-apps/api/event'

interface MeterSnapshot {
  peak_db: number
  peak_hold_db: number
  true_peak_db: number
  rms_db: number
  lufs_m: number | null
  lufs_s: number | null
  lufs_i: number | null
  clipped: boolean
}

interface TrackMeter {
  peakL: number
  peakR: number
  rms: number
}

interface MeterState {
  master: MeterSnapshot
  /// Per-track post-fader meters, keyed by track id.
  tracks: Record<string, TrackMeter>
  startListening: () => void
}

const DEFAULT_METER: MeterSnapshot = {
  peak_db: -100, peak_hold_db: -100, true_peak_db: -100,
  rms_db: -100, lufs_m: null, lufs_s: null, lufs_i: null, clipped: false,
}

export const DEFAULT_TRACK_METER: TrackMeter = { peakL: -100, peakR: -100, rms: -100 }

export const useMeterStore = create<MeterState>((set) => ({
  master: DEFAULT_METER,
  tracks: {},

  startListening: () => {
    listen<MeterSnapshot>('daw:meters', (event) => {
      set({ master: event.payload })
    })
    listen<Array<{ id: string; peakL: number; peakR: number; rms: number }>>(
      'daw:trackMeters',
      (event) => {
        const next: Record<string, TrackMeter> = {}
        for (const t of event.payload) {
          next[t.id] = { peakL: t.peakL, peakR: t.peakR, rms: t.rms }
        }
        set({ tracks: next })
      },
    )
  },
}))
