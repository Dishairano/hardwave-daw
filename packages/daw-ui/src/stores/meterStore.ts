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

interface MeterState {
  master: MeterSnapshot
  startListening: () => void
}

const DEFAULT_METER: MeterSnapshot = {
  peak_db: -100, peak_hold_db: -100, true_peak_db: -100,
  rms_db: -100, lufs_m: null, lufs_s: null, lufs_i: null, clipped: false,
}

export const useMeterStore = create<MeterState>((set) => ({
  master: DEFAULT_METER,

  startListening: () => {
    listen<MeterSnapshot>('daw:meters', (event) => {
      set({ master: event.payload })
    })
  },
}))
