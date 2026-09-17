/**
 * The project's tempo map, as the playlist needs it.
 *
 * The playlist used to draw its grid from one global numerator, so a time
 * signature set part-way through a song changed nothing on screen: the bars
 * stayed four beats wide and the bar numbers kept counting as if nothing had
 * happened. The map lives in the project, so the UI keeps a copy of it and
 * redraws from that.
 */
import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { meterSegments, type MeterSegment, type MeterEntry } from '../utils/meter'

interface TempoEntryInfo {
  tick: number
  bpm: number
  timeSigNum: number
  timeSigDen: number
  ramp: string
}

interface TempoMapState {
  entries: TempoEntryInfo[]
  /** Bar layout derived from the entries, ready for the grid to walk. */
  segments: MeterSegment[]
  refresh: () => Promise<void>
}

export const useTempoMapStore = create<TempoMapState>((set) => ({
  entries: [],
  segments: meterSegments([]),
  refresh: async () => {
    try {
      const entries = await invoke<TempoEntryInfo[]>('get_tempo_entries')
      set({ entries, segments: meterSegments(entries as MeterEntry[]) })
    } catch {
      // A failed read must not blank the grid: keeping the last known layout
      // is better than falling back to 4/4 over a 7/8 song.
    }
  },
}))
