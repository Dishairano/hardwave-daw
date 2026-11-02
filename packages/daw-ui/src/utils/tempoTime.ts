/**
 * Ticks and seconds through the song's tempo map, ramps included.
 *
 * Mirrors `TempoMap::secs_at_tick` and `tick_at_secs` in
 * `crates/hardwave-project/src/tempo.rs`. The playlist lays the song out in
 * ticks and the engine plays it in samples; where the two meet (the playhead,
 * a click on the ruler, the loop, a clip's audio) they have to convert the
 * same way, or the playhead drifts off the beat it is playing.
 *
 * The playlist used to convert at one tempo, the one the transport showed.
 * After a tempo change the playhead ran ahead of or behind the clips, and
 * while playing through a change the whole playlist rescaled under it.
 */

export const PPQ = 960

export interface TempoPoint {
  tick: number
  bpm: number
  /** 'linear' slides to the next point; anything else holds and jumps. */
  ramp: string
}

const isRamp = (p: TempoPoint) => p.ramp === 'linear'

/** The map as the engine reads it: sorted, and never empty. */
function points(entries: readonly TempoPoint[]): readonly TempoPoint[] {
  if (!entries || entries.length === 0) return [{ tick: 0, bpm: 140, ramp: 'instant' }]
  return entries
}

/** The tempo the song is at on `tick`, part-way along a ramp included. */
export function bpmAtTick(entries: readonly TempoPoint[], tick: number): number {
  const map = points(entries)
  let bpm = map[0].bpm
  for (let i = 0; i < map.length; i++) {
    const e = map[i]
    if (e.tick > tick) break
    bpm = e.bpm
    const next = map[i + 1]
    if (isRamp(e) && next && next.tick > tick && next.tick > e.tick) {
      bpm = e.bpm + (next.bpm - e.bpm) * ((tick - e.tick) / (next.tick - e.tick))
    }
  }
  return bpm
}

function ticksToSecs(ticks: number, bpm: number): number {
  return (ticks / PPQ) * (60 / bpm)
}

/** Seconds spent crossing `ticks` while the tempo slides from `b0` to `b1`. */
function rampSecs(ticks: number, b0: number, b1: number): number {
  const beats = ticks / PPQ
  if (beats <= 0 || b0 <= 0 || b1 <= 0) return 0
  if (Math.abs(b1 - b0) < 1e-9) return (beats * 60) / b0
  return (beats * 60 * Math.log(b1 / b0)) / (b1 - b0)
}

/** Ticks covered in `secs` while sliding from `b0` towards `b1` over `span`. */
function rampTicks(secs: number, b0: number, b1: number, span: number): number {
  if (secs <= 0) return 0
  const spanBeats = span / PPQ
  if (spanBeats <= 0 || b0 <= 0 || b1 <= 0) return (secs * Math.max(1, b0) / 60) * PPQ
  const k = (b1 - b0) / spanBeats
  if (Math.abs(k) < 1e-9) return (secs * b0 / 60) * PPQ
  const beats = (b0 / k) * (Math.exp((k * secs) / 60) - 1)
  return Math.min(spanBeats, Math.max(0, beats)) * PPQ
}

/** Seconds from the start of the song to `tick`. */
export function secsAtTick(entries: readonly TempoPoint[], tick: number): number {
  const map = points(entries)
  let secs = 0
  let prevTick = 0
  let prevBpm = map[0].bpm
  let prevRamp = isRamp(map[0])
  for (let i = 1; i < map.length; i++) {
    const e = map[i]
    if (e.tick >= tick) break
    const dt = e.tick - prevTick
    secs += prevRamp ? rampSecs(dt, prevBpm, e.bpm) : ticksToSecs(dt, prevBpm)
    prevTick = e.tick
    prevBpm = e.bpm
    prevRamp = isRamp(e)
  }
  const rest = tick - prevTick
  secs += prevRamp ? rampSecs(rest, prevBpm, bpmAtTick(map, tick)) : ticksToSecs(rest, prevBpm)
  return secs
}

/** The tick (with its fraction) the song reaches `secs` seconds in. */
export function tickAtSecs(entries: readonly TempoPoint[], secs: number): number {
  const map = points(entries)
  const target = Math.max(0, secs)
  let acc = 0
  let prevTick = 0
  let prevBpm = map[0].bpm
  let prevRamp = isRamp(map[0])
  let nextBpm = prevBpm
  let span = 0
  for (let i = 1; i < map.length; i++) {
    const e = map[i]
    const dt = e.tick - prevTick
    const seg = prevRamp ? rampSecs(dt, prevBpm, e.bpm) : ticksToSecs(dt, prevBpm)
    if (acc + seg > target) {
      nextBpm = e.bpm
      span = dt
      break
    }
    acc += seg
    prevTick = e.tick
    prevBpm = e.bpm
    prevRamp = isRamp(e)
    nextBpm = e.bpm
    span = 0
  }
  const rest = target - acc
  const ticks = prevRamp && span > 0
    ? rampTicks(rest, prevBpm, nextBpm, span)
    : (rest * prevBpm / 60) * PPQ
  return prevTick + ticks
}

/** Where a sample position is on the song's beat grid, in ticks. */
export function tickAtSample(entries: readonly TempoPoint[], samples: number, sampleRate: number): number {
  return sampleRate > 0 ? tickAtSecs(entries, samples / sampleRate) : 0
}

/** The sample a tick is heard on. */
export function sampleAtTick(entries: readonly TempoPoint[], tick: number, sampleRate: number): number {
  return Math.round(secsAtTick(entries, tick) * (sampleRate > 0 ? sampleRate : 48000))
}

/**
 * The tempo the playlist is laid out at: the song's opening tempo. The
 * playlist is in bars and beats, so its scale must not move while the song
 * plays through a tempo change; at zoom 1 an opening beat is as wide as it
 * always was.
 */
export function layoutBpm(entries: readonly TempoPoint[], fallback = 140): number {
  const first = entries?.[0]?.bpm
  return first && first > 0 ? first : fallback
}

/** The tempo over `from..to` when it holds still there, else null. */
export function steadyBpmBetween(entries: readonly TempoPoint[], from: number, to: number): number | null {
  const map = points(entries)
  const start = bpmAtTick(map, from)
  if (to <= from) return start
  for (let i = 0; i < map.length; i++) {
    const e = map[i]
    const next = map[i + 1]
    const overlaps = e.tick < to && (!next || next.tick > from)
    if (!overlaps) continue
    if (Math.abs(e.bpm - start) > 1e-9) return null
    if (isRamp(e) && next && Math.abs(next.bpm - e.bpm) > 1e-9) return null
  }
  return start
}
