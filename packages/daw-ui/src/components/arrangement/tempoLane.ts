/**
 * The tempo lane under the playlist's ruler: the song's tempo map drawn as a
 * line, a point at each change and a slope where it ramps. Approved from the
 * tempo lane mockup (suite: daw-tempo-lane-mockup), item A7 of finishing the
 * DAW.
 *
 * What is here is the drawing and the geometry, kept apart from the canvas
 * and the mouse so it can be tested; `TempoLane.tsx` handles the gestures.
 */
import type { TempoPoint } from '../../utils/tempoTime'
import type { MeterSegment } from '../../utils/meter'

export const TEMPO_LANE_HEIGHT = 74
export const TEMPO_COLOR = '#f5b041'
/** Room above and below the line, so a point at the edge stays grabbable. */
const PAD = 10

export interface LaneScale {
  lo: number
  hi: number
}

/** The lane's range of tempos: the song's own, with room around it, so a
 *  steady song sits in the middle and a ramp fills most of the height. */
export function laneScale(entries: readonly TempoPoint[]): LaneScale {
  const bpms = entries.length ? entries.map((e) => e.bpm) : [140]
  const min = Math.min(...bpms)
  const max = Math.max(...bpms)
  const span = Math.max(10, max - min)
  return { lo: Math.max(10, min - span * 0.45), hi: max + span * 0.45 }
}

export function bpmToY(bpm: number, scale: LaneScale, height = TEMPO_LANE_HEIGHT): number {
  const t = (bpm - scale.lo) / Math.max(1e-6, scale.hi - scale.lo)
  return PAD + (1 - t) * (height - PAD * 2)
}

export function yToBpm(y: number, scale: LaneScale, height = TEMPO_LANE_HEIGHT): number {
  const t = 1 - (y - PAD) / Math.max(1, height - PAD * 2)
  return scale.lo + t * (scale.hi - scale.lo)
}

/** A tempo as the lane writes it: a tenth of a BPM, inside what the engine
 *  takes. */
export function roundBpm(bpm: number): number {
  return Math.min(999, Math.max(20, Math.round(bpm * 10) / 10))
}

/** Tempos to draw a faint guide at: round numbers inside the scale. */
export function guideValues(scale: LaneScale): number[] {
  const span = scale.hi - scale.lo
  const step = span > 120 ? 50 : span > 60 ? 20 : span > 24 ? 10 : 5
  const out: number[] = []
  for (let v = Math.ceil(scale.lo / step) * step; v < scale.hi; v += step) out.push(v)
  return out
}

/**
 * The point under the pointer, or null. When two points sit on one spot (a
 * ramp's end and the jump after it) the nearer wins, and on a tie the later
 * one, which is the one just added there.
 */
export function pointAt(
  entries: readonly TempoPoint[],
  x: number,
  y: number,
  tickToX: (tick: number) => number,
  scale: LaneScale,
  height = TEMPO_LANE_HEIGHT,
  reach = 8,
): number | null {
  let best: number | null = null
  let bestD = reach * reach
  entries.forEach((e, i) => {
    const dx = tickToX(e.tick) - x
    const dy = bpmToY(e.bpm, scale, height) - y
    const d = dx * dx + dy * dy
    if (d <= bestD) {
      best = i
      bestD = d
    }
  })
  return best
}

/** The change in force at `tick`: the last point at or before it. */
export function changeAt(entries: readonly TempoPoint[], tick: number): number {
  let index = 0
  entries.forEach((e, i) => { if (e.tick <= tick) index = i })
  return index
}

/** How far a point may move sideways: not onto or past its neighbours, and
 *  the song's opening tempo stays at the start. */
export function clampTick(entries: readonly TempoPoint[], index: number, tick: number): number {
  if (index <= 0) return 0
  const prev = entries[index - 1]
  const next = entries[index + 1]
  const lo = prev ? prev.tick + 1 : 1
  const hi = next ? next.tick - 1 : Number.MAX_SAFE_INTEGER
  return Math.max(lo, Math.min(hi, Math.round(tick)))
}

/** The song's tempos in order, as the lane's name shows them:
 *  "150 → 160 → 150". */
export function tempoSummary(entries: readonly TempoPoint[], most = 4): string {
  const values: number[] = []
  for (const e of entries) {
    const v = Math.round(e.bpm * 10) / 10
    if (values.length === 0 || values[values.length - 1] !== v) values.push(v)
  }
  if (values.length === 0) return '140'
  const text = values.slice(0, most).map((v) => (v % 1 === 0 ? v.toFixed(0) : v.toFixed(1)))
  return text.join(' → ') + (values.length > most ? ' …' : '')
}

/** Where a tick is, as the lane names it: "bar 17", or "bar 17 : 3" off
 *  the bar line. A ramp's end a tick before a jump reads as the jump's bar. */
export function barLabel(segments: readonly MeterSegment[], tick: number): string {
  const at = Math.round(tick / 60) * 60
  let seg = segments[0]
  for (const s of segments) if (s.startTick <= at) seg = s
  if (!seg) return `bar ${Math.floor(at / 3840) + 1}`
  const into = Math.max(0, at - seg.startTick)
  const bar = seg.startBar + Math.floor(into / seg.ticksPerBar)
  const beat = Math.floor((into % seg.ticksPerBar) / Math.max(1, seg.ticksPerBeat)) + 1
  const onBar = into % seg.ticksPerBar === 0
  return onBar ? `bar ${bar}` : `bar ${bar} : ${beat}`
}

export interface LaneDraw {
  entries: readonly TempoPoint[]
  top: number
  width: number
  height?: number
  pxPerTick: number
  scrollOffset: number
  scale: LaneScale
  /** The point being dragged or pointed at, drawn large. */
  hot: number | null
  /** The bar and beat lines of the grid below, so the lane reads against
   *  the same bars. */
  lines: readonly { tick: number; isBar: boolean }[]
}

/** Draw the lane on the playlist's canvas. */
export function drawTempoLane(ctx: CanvasRenderingContext2D, d: LaneDraw): void {
  const height = d.height ?? TEMPO_LANE_HEIGHT
  const { top, width: w, scale } = d
  const x = (tick: number) => tick * d.pxPerTick - d.scrollOffset
  const y = (bpm: number) => top + bpmToY(bpm, scale, height)

  ctx.save()
  ctx.beginPath()
  ctx.rect(0, top, w, height)
  ctx.clip()
  // Solid underneath, so clips scrolled up under the lane do not show
  // through, with the lane's own amber wash over it.
  ctx.fillStyle = '#0e0e11'
  ctx.fillRect(0, top, w, height)
  const wash = ctx.createLinearGradient(0, top, 0, top + height)
  wash.addColorStop(0, 'rgba(245,176,65,0.05)')
  wash.addColorStop(1, 'rgba(245,176,65,0.01)')
  ctx.fillStyle = wash
  ctx.fillRect(0, top, w, height)

  for (const line of d.lines) {
    const lx = Math.floor(x(line.tick)) + 0.5
    if (lx < -1 || lx > w + 1) continue
    ctx.strokeStyle = line.isBar ? 'rgba(255,255,255,0.1)' : 'rgba(255,255,255,0.035)'
    ctx.lineWidth = 1
    ctx.beginPath()
    ctx.moveTo(lx, top)
    ctx.lineTo(lx, top + height)
    ctx.stroke()
  }

  // Guides at round tempos, named at the right edge.
  ctx.font = '500 9px "JetBrains Mono", ui-monospace, monospace'
  ctx.textBaseline = 'alphabetic'
  ctx.setLineDash([2, 4])
  for (const v of guideValues(scale)) {
    const gy = Math.round(y(v)) + 0.5
    ctx.strokeStyle = 'rgba(245,176,65,0.08)'
    ctx.beginPath()
    ctx.moveTo(0, gy)
    ctx.lineTo(w, gy)
    ctx.stroke()
    ctx.fillStyle = 'rgba(245,176,65,0.45)'
    ctx.fillText(String(v), w - 26, gy - 2)
  }
  ctx.setLineDash([])

  // The line: a slope where a point ramps to the next, a step where it
  // holds and jumps.
  const entries = d.entries.length ? d.entries : [{ tick: 0, bpm: 140, ramp: 'instant' }]
  const path = new Path2D()
  path.moveTo(Math.min(0, x(entries[0].tick)), y(entries[0].bpm))
  entries.forEach((e, i) => {
    const next = entries[i + 1]
    path.lineTo(x(e.tick), y(e.bpm))
    if (!next) {
      path.lineTo(w, y(e.bpm))
    } else if (e.ramp === 'linear') {
      path.lineTo(x(next.tick), y(next.bpm))
    } else {
      path.lineTo(x(next.tick), y(e.bpm))
    }
  })
  const fill = new Path2D(path)
  fill.lineTo(w, top + height)
  fill.lineTo(Math.min(0, x(entries[0].tick)), top + height)
  fill.closePath()
  ctx.fillStyle = 'rgba(245,176,65,0.07)'
  ctx.fill(fill)
  ctx.strokeStyle = TEMPO_COLOR
  ctx.lineWidth = 2
  ctx.lineJoin = 'round'
  ctx.stroke(path)

  entries.forEach((e, i) => {
    const px = x(e.tick)
    if (px < -8 || px > w + 8) return
    const hot = i === d.hot
    ctx.beginPath()
    ctx.arc(px, y(e.bpm), hot ? 6 : 4.5, 0, Math.PI * 2)
    ctx.fillStyle = hot ? TEMPO_COLOR : '#0b0b0e'
    ctx.fill()
    ctx.strokeStyle = TEMPO_COLOR
    ctx.lineWidth = 2
    ctx.stroke()
  })

  ctx.strokeStyle = 'rgba(255,255,255,0.08)'
  ctx.lineWidth = 1
  ctx.beginPath()
  ctx.moveTo(0, top + height - 0.5)
  ctx.lineTo(w, top + height - 0.5)
  ctx.stroke()
  ctx.restore()
}
