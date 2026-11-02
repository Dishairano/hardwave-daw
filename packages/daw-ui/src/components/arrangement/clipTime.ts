/**
 * Where an audio clip's file is drawn along the playlist, so the waveform
 * shows what the engine plays under the song's tempo map.
 *
 * A clip that follows the tempo has its file laid over the beats at a fixed
 * rate (`AudioClip::file_frames_per_tick`): a straight line in ticks,
 * whatever the tempo does. A clip that keeps its speed plays its file in
 * seconds from the beat it starts on, so across a tempo change its waveform
 * is squeezed where the song is faster. It used to be drawn at the tempo the
 * transport showed, which matched neither once the tempo moved.
 */
import { PPQ, bpmAtTick, secsAtTick, steadyBpmBetween, tickAtSecs, type TempoPoint } from '../../utils/tempoTime'

export interface FileLayout {
  /** Pixels from the clip's anchor tick to second `fileSec` of its file. */
  xAt: (fileSec: number) => number
  /** The second of the file drawn `dx` pixels from the anchor. */
  secAt: (dx: number) => number
  /** Pixels per second of the file, where the layout is a straight line;
   *  at the anchor where it is not. */
  pxPerSec: number
  linear: boolean
}

export interface FileLayoutInput {
  entries: readonly TempoPoint[]
  /** The tick the clip's file is counted from: its start, or where its
   *  start was while its left edge is being dragged. */
  anchorTick: number
  lengthTicks: number
  pxPerTick: number
  /** Where in the file the clip starts, in seconds. */
  sourceStartSec: number
  /** How much longer than its file the sample lasts (stretch, and in
   *  resample the pitch). */
  timeFactor: number
  /** The tempo the file is in when the clip follows the tempo. */
  followBpm?: number | null
  /** Warp markers pin the clip to ticks by themselves. */
  warped?: boolean
}

export function fileLayout(c: FileLayoutInput): FileLayout {
  const tf = c.timeFactor > 0 ? c.timeFactor : 1
  const start = c.sourceStartSec
  if (c.followBpm && c.followBpm > 0 && !c.warped) {
    const pxPerSec = ((PPQ * c.followBpm * tf) / 60) * c.pxPerTick
    return linear(start, pxPerSec)
  }
  const steady = steadyBpmBetween(c.entries, c.anchorTick, c.anchorTick + Math.max(1, c.lengthTicks))
  const startSecs = secsAtTick(c.entries, c.anchorTick)
  if (steady !== null) {
    return linear(start, tf * c.pxPerTick * PPQ * (steady / 60))
  }
  const pxPerSecAtStart = tf * c.pxPerTick * PPQ * (bpmAtTick(c.entries, c.anchorTick) / 60)
  return {
    xAt: (s) => (tickAtSecs(c.entries, startSecs + (s - start) * tf) - c.anchorTick) * c.pxPerTick,
    secAt: (dx) => (secsAtTick(c.entries, c.anchorTick + dx / c.pxPerTick) - startSecs) / tf + start,
    pxPerSec: pxPerSecAtStart,
    linear: false,
  }
}

function linear(start: number, pxPerSec: number): FileLayout {
  return {
    xAt: (s) => (s - start) * pxPerSec,
    secAt: (dx) => start + dx / pxPerSec,
    pxPerSec,
    linear: true,
  }
}

/**
 * What a clip that follows the tempo says about its speed: "stretched 1.07",
 * or "stretched 1.00 → 1.07" across a ramp. Null when it plays at its own
 * speed throughout, or does not follow the tempo.
 */
export function stretchLabel(
  entries: readonly TempoPoint[],
  positionTicks: number,
  lengthTicks: number,
  followBpm: number | null | undefined,
  timeFactor: number,
): string | null {
  if (!followBpm || followBpm <= 0) return null
  const tf = timeFactor > 0 ? timeFactor : 1
  const speed = (tick: number) => bpmAtTick(entries, tick) / (followBpm * tf)
  const a = speed(positionTicks)
  const b = speed(positionTicks + Math.max(1, lengthTicks) - 1)
  const same = (x: number, y: number) => Math.abs(x - y) < 0.005
  if (same(a, 1) && same(b, 1)) return null
  return same(a, b) ? `stretched ${a.toFixed(2)}` : `stretched ${a.toFixed(2)} → ${b.toFixed(2)}`
}
