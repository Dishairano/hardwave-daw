/**
 * Bar and beat layout for a project that changes time signature part-way
 * through.
 *
 * Mirrors `TempoMap::meter_segments` and `tick_to_bar_beat` in
 * `crates/hardwave-project/src/tempo.rs`. The rules have to match, because
 * the engine counts the bars the click accents and this counts the bars the
 * user sees; if they disagree, the click lands off the grid it is drawn on.
 */

export const PPQ = 960

export interface MeterEntry {
  tick: number
  timeSigNum: number
  timeSigDen: number
}

export interface MeterSegment {
  /** Tick this signature starts at. */
  startTick: number
  /** Bar number at `startTick`, 1-indexed. */
  startBar: number
  ticksPerBar: number
  ticksPerBeat: number
  timeSigNum: number
  timeSigDen: number
}

/**
 * Ticks in one beat: a quarter note for /4, an eighth for /8.
 *
 * The denominator used to be ignored, so a bar of 7/8 was drawn seven quarter
 * notes wide, twice as long as it sounded.
 */
export function ticksPerBeat(den: number): number {
  if (!Number.isFinite(den) || den <= 0) return PPQ
  return (PPQ * 4) / den
}

export function ticksPerBar(num: number, den: number): number {
  return Math.max(1, ticksPerBeat(den) * Math.max(1, num))
}

/**
 * Where the bars restart.
 *
 * Only an entry that changes the signature starts a new bar. A tempo change
 * does not: tempo points can sit anywhere, and treating each one as a bar
 * line would shift the whole grid when someone added a tempo point half way
 * through a bar.
 */
export function meterSegments(entries: MeterEntry[]): MeterSegment[] {
  const changes: MeterEntry[] = []
  let current: string | null = null
  // A backend that answers with nothing must still leave a drawable grid,
  // rather than throwing inside the canvas draw.
  const sorted = Array.isArray(entries) ? [...entries].sort((a, b) => a.tick - b.tick) : []
  for (const entry of sorted) {
    const num = Math.max(1, Math.floor(entry.timeSigNum || 4))
    const den = Math.max(1, Math.floor(entry.timeSigDen || 4))
    const key = `${num}/${den}`
    if (key === current) continue
    current = key
    changes.push({ tick: entry.tick, timeSigNum: num, timeSigDen: den })
  }
  if (changes.length === 0) changes.push({ tick: 0, timeSigNum: 4, timeSigDen: 4 })
  // A signature change before the first bar line would leave the song
  // starting mid-bar, so the first segment always counts from zero.
  changes[0].tick = 0

  const segments: MeterSegment[] = []
  let bar = 1
  for (let i = 0; i < changes.length; i++) {
    const change = changes[i]
    const perBar = ticksPerBar(change.timeSigNum, change.timeSigDen)
    segments.push({
      startTick: change.tick,
      startBar: bar,
      ticksPerBar: perBar,
      ticksPerBeat: ticksPerBeat(change.timeSigDen),
      timeSigNum: change.timeSigNum,
      timeSigDen: change.timeSigDen,
    })
    const next = changes[i + 1]
    if (next) {
      // A signature change starts a new bar, so a segment that does not
      // divide evenly still consumes the bar it cut short.
      bar += Math.ceil((next.tick - change.tick) / perBar)
    }
  }
  return segments
}

/** The segment in force at a tick. */
export function segmentAt(segments: MeterSegment[], tick: number): MeterSegment {
  let found = segments[0]
  for (const segment of segments) {
    if (segment.startTick > tick) break
    found = segment
  }
  return found
}

export interface GridLine {
  tick: number
  /** True on a bar line, false on a plain beat. */
  isBar: boolean
  /** Bar number, 1-indexed, for a bar line. */
  bar: number
}

/**
 * Every beat line between two ticks, in order.
 *
 * Walks segment by segment so the beat width and the bar length change where
 * the signature does, which a single "tick % ticksPerBar" test cannot do.
 * `maxLines` is a safety valve: at the widest zoom a long song is tens of
 * thousands of beats, and the canvas only needs what fits on it.
 */
export function gridLines(
  segments: MeterSegment[],
  fromTick: number,
  toTick: number,
  maxLines = 4000,
): GridLine[] {
  const lines: GridLine[] = []
  if (segments.length === 0 || toTick <= fromTick) return lines

  for (let i = 0; i < segments.length; i++) {
    const segment = segments[i]
    const next = segments[i + 1]
    const segmentEnd = next ? next.startTick : Number.POSITIVE_INFINITY
    if (segmentEnd <= fromTick) continue
    if (segment.startTick > toTick) break

    const perBeat = Math.max(1, segment.ticksPerBeat)
    const beatsPerBar = Math.max(1, Math.round(segment.ticksPerBar / perBeat))
    const firstBeat = Math.max(
      0,
      Math.floor((Math.max(fromTick, segment.startTick) - segment.startTick) / perBeat),
    )

    for (let beat = firstBeat; ; beat++) {
      const tick = segment.startTick + beat * perBeat
      if (tick > toTick || tick >= segmentEnd) break
      lines.push({
        tick,
        isBar: beat % beatsPerBar === 0,
        bar: segment.startBar + Math.floor(beat / beatsPerBar),
      })
      if (lines.length >= maxLines) return lines
    }
  }
  return lines
}

/** Bar and beat at a tick, both 1-indexed, for a read-out. */
export function barBeatAtTick(segments: MeterSegment[], tick: number): { bar: number; beat: number } {
  const segment = segmentAt(segments, tick)
  const into = Math.max(0, tick - segment.startTick)
  return {
    bar: segment.startBar + Math.floor(into / segment.ticksPerBar),
    beat: (into % segment.ticksPerBar) / Math.max(1, segment.ticksPerBeat) + 1,
  }
}
