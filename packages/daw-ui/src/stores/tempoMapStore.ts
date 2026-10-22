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
import { useHistoryStore } from './historyStore'
import { barStartAt, bpmAt } from '../components/arrangement/tempoFlags'

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
  /** A tempo change at `tick`: the entry already there changes, and at the
   *  song's start the opening tempo does. */
  setTempoAt: (tick: number, bpm: number) => Promise<void>
  /** A signature change from the bar line at or before `tick`. */
  setSignatureAt: (tick: number, num: number, den: number) => Promise<void>
  setEntryTempo: (index: number, bpm: number) => Promise<void>
  setEntrySignature: (index: number, num: number, den: number) => Promise<void>
  /** Take a change out: the song goes on at what came before it. */
  removeTempoChange: (index: number) => Promise<void>
  removeSignatureChange: (index: number) => Promise<void>
}

/** Several tempo-map edits that make one change, as one undo step. */
async function asOneStep(label: string, edit: () => Promise<void>): Promise<void> {
  await invoke('begin_history_group').catch(() => {})
  try {
    await edit()
  } finally {
    await invoke('end_history_group').catch(() => {})
    useHistoryStore.getState().push(label)
  }
}

async function fresh(): Promise<TempoEntryInfo[]> {
  return invoke<TempoEntryInfo[]>('get_tempo_entries')
}

export const useTempoMapStore = create<TempoMapState>((set, get) => ({
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

  setTempoAt: async (tick, bpm) => {
    await asOneStep(`Tempo ${bpm} BPM`, async () => {
      const entries = await fresh()
      const at = tick === 0 ? 0 : entries.findIndex((e) => e.tick === tick)
      if (at >= 0) {
        await invoke('set_tempo_entry', { index: at, tick: entries[at].tick, bpm, ramp: entries[at].ramp })
      } else {
        await invoke('add_tempo_entry', { tick, bpm, ramp: 'instant' })
      }
    })
    await get().refresh()
  },

  setSignatureAt: async (tick, num, den) => {
    const at = barStartAt(get().segments, tick)
    await asOneStep(`Time signature ${num}/${den}`, async () => {
      let entries = await fresh()
      let index = entries.findIndex((e) => e.tick === at)
      if (index < 0) {
        // A signature lives on a tempo-map entry: one at the tempo already
        // playing there, so only the bars change.
        await invoke('add_tempo_entry', { tick: at, bpm: bpmAt(entries, at), ramp: 'instant' })
        entries = await fresh()
        index = entries.findIndex((e) => e.tick === at)
      }
      await invoke('set_tempo_entry_time_signature', { index, numerator: num, denominator: den })
    })
    await get().refresh()
  },

  setEntryTempo: async (index, bpm) => {
    const e = (await fresh())[index]
    if (!e) return
    await asOneStep(`Tempo ${bpm} BPM`, () =>
      invoke('set_tempo_entry', { index, tick: e.tick, bpm, ramp: e.ramp }))
    await get().refresh()
  },

  setEntrySignature: async (index, num, den) => {
    await asOneStep(`Time signature ${num}/${den}`, () =>
      invoke('set_tempo_entry_time_signature', { index, numerator: num, denominator: den }))
    await get().refresh()
  },

  removeTempoChange: async (index) => {
    const entries = await fresh()
    const e = entries[index]
    const before = entries[index - 1]
    if (!e || !before) return
    await asOneStep('Remove tempo change', async () => {
      if (e.timeSigNum !== before.timeSigNum || e.timeSigDen !== before.timeSigDen) {
        // The entry also starts a new signature: keep that, drop the tempo.
        await invoke('set_tempo_entry', { index, tick: e.tick, bpm: before.bpm, ramp: e.ramp })
      } else {
        await invoke('remove_tempo_entry', { index })
      }
    })
    await get().refresh()
  },

  removeSignatureChange: async (index) => {
    const entries = await fresh()
    const e = entries[index]
    const before = entries[index - 1]
    if (!e || !before) return
    await asOneStep('Remove time signature change', async () => {
      if (e.bpm !== before.bpm) {
        await invoke('set_tempo_entry_time_signature', {
          index, numerator: before.timeSigNum, denominator: before.timeSigDen,
        })
      } else {
        await invoke('remove_tempo_entry', { index })
      }
    })
    await get().refresh()
  },
}))
