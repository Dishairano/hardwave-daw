// The ruler's tempo and time-signature flags, read from the song's tempo map.
//
// The ruler's "Add tempo change" and "Add time signature change" used to
// drop a flag in the marker list and nothing else: the song kept its tempo
// and its bars, so the flag said one thing and the song played another.
// The flags are now drawn from the tempo map itself, so what the ruler shows
// is what plays.

import type { Marker } from '../../stores/markerStore'
import type { MeterSegment } from '../../utils/meter'

export interface TempoMapEntry {
  tick: number
  bpm: number
  timeSigNum: number
  timeSigDen: number
  ramp: string
}

export const TEMPO_FLAG_COLOR = '#F97316'
export const TIMESIG_FLAG_COLOR = '#14B8A6'

/** One flag where the tempo changes and one where the signature does. Ids
 *  carry the entry's index: "tempo:3", "timesig:3". The song's opening tempo
 *  and signature are not changes and get no flag. */
export function tempoFlags(entries: TempoMapEntry[]): Marker[] {
  const flags: Marker[] = []
  entries.forEach((e, i) => {
    if (i === 0) return
    const before = entries[i - 1]
    if (e.bpm !== before.bpm) {
      flags.push({
        id: `tempo:${i}`, tick: e.tick, kind: 'tempo', bpm: e.bpm,
        label: `${e.bpm % 1 === 0 ? e.bpm.toFixed(0) : e.bpm.toFixed(2)} BPM`, color: TEMPO_FLAG_COLOR,
      })
    }
    if (e.timeSigNum !== before.timeSigNum || e.timeSigDen !== before.timeSigDen) {
      flags.push({
        id: `timesig:${i}`, tick: e.tick, kind: 'timesig', timeSigNum: e.timeSigNum, timeSigDen: e.timeSigDen,
        label: `${e.timeSigNum}/${e.timeSigDen}`, color: TIMESIG_FLAG_COLOR,
      })
    }
  })
  return flags
}

/** The tempo-map entry a flag stands for, or null for an ordinary marker. */
export function flagEntry(id: string): { kind: 'tempo' | 'timesig'; index: number } | null {
  const m = /^(tempo|timesig):(\d+)$/.exec(id)
  return m ? { kind: m[1] as 'tempo' | 'timesig', index: Number(m[2]) } : null
}

/** Where the bar holding `tick` starts: a signature change sits on a bar line. */
export function barStartAt(segments: MeterSegment[], tick: number): number {
  let seg = segments[0]
  for (const s of segments) if (s.startTick <= tick) seg = s
  if (!seg || seg.ticksPerBar <= 0) return Math.max(0, tick)
  const bars = Math.floor((tick - seg.startTick) / seg.ticksPerBar)
  return seg.startTick + Math.max(0, bars) * seg.ticksPerBar
}

/** The tempo the song is at on `tick`, as the last entry before it set it. */
export function bpmAt(entries: TempoMapEntry[], tick: number): number {
  let bpm = entries[0]?.bpm ?? 140
  for (const e of entries) if (e.tick <= tick) bpm = e.bpm
  return bpm
}
