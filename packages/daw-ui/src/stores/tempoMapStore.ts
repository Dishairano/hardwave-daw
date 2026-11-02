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
import { bpmAtTick, sampleAtTick, tickAtSample } from '../utils/tempoTime'
import type { LaneScale } from '../components/arrangement/tempoLane'

export interface TempoEntryInfo {
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

  // The tempo lane.
  /** The point being dragged or pointed at, drawn large. */
  laneHot: number | null
  /** The lane's tempo range, held still while a point is dragged so the
   *  point stays under the pointer. Null: fit to the song. */
  laneScale: LaneScale | null
  setLaneHot: (index: number | null) => void
  setLaneScale: (scale: LaneScale | null) => void
  /** Show a point where a drag has put it, before the engine has it. */
  previewEntry: (index: number, tick: number, bpm: number) => void
  /** A tempo change at `tick`, at the tempo playing there. On a point that
   *  is already there it becomes a pair: the old one moves a tick earlier
   *  and ends what came before, the new one starts what comes after, which
   *  is how a ramp ends in a jump. Returns the new point's index. */
  addTempoChange: (tick: number) => Promise<number | null>
  /** Whether the tempo slides from a point to the next ('linear') or holds
   *  and jumps there ('instant'). */
  setEntryRamp: (index: number, ramp: 'linear' | 'instant') => Promise<void>
}

/** Several tempo-map edits that make one change, as one undo step. */
async function asOneStep(label: string, edit: () => Promise<void>): Promise<void> {
  const since = Date.now()
  await invoke('begin_history_group').catch(() => {})
  try {
    await edit()
  } finally {
    await invoke('end_history_group').catch(() => {})
    useHistoryStore.getState().push(label, since)
  }
  void tempoChanged()
}

/** Where a sample position is on the song's bars, through its tempo map:
 *  for anything that puts something "at the playhead". One tempo put it in
 *  the wrong bar after a tempo change. */
export function tickOfSample(samples: number, sampleRate: number): number {
  return tickAtSample(useTempoMapStore.getState().entries, samples, sampleRate || 48000)
}

/** The sample a tick is heard on, through the song's tempo map. */
export function sampleOfTick(tick: number, sampleRate: number): number {
  return sampleAtTick(useTempoMapStore.getState().entries, tick, sampleRate || 48000)
}

let tracksTimer: ReturnType<typeof setTimeout> | null = null
/**
 * After the tempo map changed: read the clips again. Audio that keeps its
 * speed is a different number of ticks long under the new tempo, and the
 * engine has worked that out; the playlist has to show it. Gathered, so a
 * drag over the tempo field reads them once it settles rather than on every
 * step.
 */
export function tempoChanged(): void {
  if (tracksTimer) clearTimeout(tracksTimer)
  tracksTimer = setTimeout(() => {
    tracksTimer = null
    void useTempoMapStore.getState().refresh()
    void import('./trackStore').then(({ useTrackStore }) => useTrackStore.getState().fetchTracks()).catch(() => {})
  }, 120)
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

  laneHot: null,
  laneScale: null,
  setLaneHot: (laneHot) => { if (get().laneHot !== laneHot) set({ laneHot }) },
  setLaneScale: (laneScale) => set({ laneScale }),

  previewEntry: (index, tick, bpm) => {
    const entries = get().entries.slice()
    const e = entries[index]
    if (!e) return
    entries[index] = { ...e, tick, bpm }
    set({ entries })
  },

  addTempoChange: async (tick) => {
    const at = Math.max(0, Math.round(tick))
    if (at === 0) return null
    const added = { index: -1 }
    await asOneStep('Add tempo change', async () => {
      const entries = await fresh()
      const here = entries.findIndex((e) => e.tick === at)
      if (here >= 0) {
        const e = entries[here]
        const before = entries[here - 1]
        if (!before || before.tick >= at - 1) return
        // The pair: the old point ends what came before a tick early and
        // jumps; the new one carries on where it led.
        await invoke('set_tempo_entry', { index: here, tick: at - 1, bpm: e.bpm, ramp: 'instant' })
        await invoke('add_tempo_entry', { tick: at, bpm: e.bpm, ramp: e.ramp })
      } else {
        const inForce = entries.filter((e) => e.tick <= at).pop()
        const bpm = Math.round(bpmAtTick(entries, at) * 10) / 10
        await invoke('add_tempo_entry', { tick: at, bpm, ramp: inForce?.ramp ?? 'instant' })
      }
      added.index = (await fresh()).findIndex((e) => e.tick === at)
    })
    await get().refresh()
    return added.index >= 0 ? added.index : null
  },

  setEntryRamp: async (index, ramp) => {
    const e = (await fresh())[index]
    if (!e || e.ramp === ramp) return
    await asOneStep(ramp === 'linear' ? 'Ramp to the next tempo' : 'Jump at the next tempo', () =>
      invoke('set_tempo_entry', { index, tick: e.tick, bpm: e.bpm, ramp }))
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
