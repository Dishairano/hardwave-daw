import { useMeterStore } from '../stores/meterStore'

/**
 * Single global meter painter.
 *
 * Replaces the per-component `useTrackMeter(id)` selector pattern for the
 * `<Meter>` primitive. The selector approach forces a React re-render on
 * every meter tick — at 60 Hz × 500 strips that's 30 000 React updates per
 * second and burns the renderer thread.
 *
 * This singleton owns:
 *  - A `Map<HTMLCanvasElement, MeterRegistration>` of every visible meter
 *  - A single `requestAnimationFrame` loop that reads the meter store's
 *    current values via `getState()` (no subscription, no re-render) and
 *    paints each canvas directly via the 2D context.
 *
 * Meter components call `register(canvas, trackId, channel)` on mount and
 * `unregister(canvas)` on unmount. The rAF loop starts on the first
 * registration and stops itself when the last meter unregisters.
 *
 * Why not a Worker? Tauri's WebView2 (Windows) and WKWebView (macOS) don't
 * always serve the COOP/COEP headers needed for SharedArrayBuffer + an
 * OffscreenCanvas controlled by a worker. Main-thread rAF still buys the
 * "no React re-renders" win, which is where 80% of the cost was. If we
 * ever need the last 20% we can layer a worker on top without changing
 * the public API of this module.
 */

interface MeterRegistration {
  trackId: string
  channel: 'l' | 'r' | 'mono'
  minDb: number
  maxDb: number
  lastPaintedFill: number
}

const META = new WeakMap<HTMLCanvasElement, MeterRegistration>()
const ACTIVE = new Set<HTMLCanvasElement>()

let rafId: number | null = null

const GRADIENT_STOPS: Array<[number, string]> = [
  [0.0, '#3ed07a'],
  [0.55, '#3ed07a'],
  [0.75, '#f0a032'],
  [0.92, '#ff2d4f'],
  [1.0, '#ff2d4f'],
]

function clamp(v: number, lo: number, hi: number): number {
  if (v < lo) return lo
  if (v > hi) return hi
  return v
}

function buildGradient(ctx: CanvasRenderingContext2D, h: number): CanvasGradient {
  const g = ctx.createLinearGradient(0, h, 0, 0)
  for (const [stop, color] of GRADIENT_STOPS) g.addColorStop(stop, color)
  return g
}

function paintCanvas(canvas: HTMLCanvasElement, reg: MeterRegistration, fillPct: number) {
  // Skip paint when the value hasn't moved enough to be visible — saves
  // ~30% of canvas draw calls when a track is mostly silent.
  if (Math.abs(fillPct - reg.lastPaintedFill) < 0.4) return
  reg.lastPaintedFill = fillPct
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  const dpr = window.devicePixelRatio || 1
  const cssW = canvas.clientWidth
  const cssH = canvas.clientHeight
  // Resize the canvas backing store when its visible size changes.
  const wantW = Math.max(1, Math.round(cssW * dpr))
  const wantH = Math.max(1, Math.round(cssH * dpr))
  if (canvas.width !== wantW || canvas.height !== wantH) {
    canvas.width = wantW
    canvas.height = wantH
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx.fillStyle = '#08080a'
  ctx.fillRect(0, 0, cssW, cssH)
  const fillH = (clamp(fillPct, 0, 100) / 100) * cssH
  if (fillH > 0.5) {
    ctx.fillStyle = buildGradient(ctx, cssH)
    ctx.fillRect(0, cssH - fillH, cssW, fillH)
  }
}

function tick() {
  if (ACTIVE.size === 0) {
    rafId = null
    return
  }
  const trackMeters = useMeterStore.getState().tracks
  for (const canvas of ACTIVE) {
    const reg = META.get(canvas)
    if (!reg) continue
    const meter = trackMeters[reg.trackId]
    if (!meter) {
      // Track removed mid-session — paint silence and skip.
      paintCanvas(canvas, reg, 0)
      continue
    }
    const peakDb =
      reg.channel === 'l' ? meter.peakL : reg.channel === 'r' ? meter.peakR : (meter.peakL + meter.peakR) / 2
    const range = reg.maxDb - reg.minDb
    const fillPct = range > 0 ? ((clamp(peakDb, reg.minDb, reg.maxDb) - reg.minDb) / range) * 100 : 0
    paintCanvas(canvas, reg, fillPct)
  }
  rafId = requestAnimationFrame(tick)
}

export function registerMeter(
  canvas: HTMLCanvasElement,
  trackId: string,
  channel: 'l' | 'r' | 'mono',
  minDb = -60,
  maxDb = 6,
): void {
  META.set(canvas, { trackId, channel, minDb, maxDb, lastPaintedFill: -1 })
  ACTIVE.add(canvas)
  if (rafId == null) rafId = requestAnimationFrame(tick)
}

export function unregisterMeter(canvas: HTMLCanvasElement): void {
  ACTIVE.delete(canvas)
  META.delete(canvas)
  // The tick loop self-terminates next frame when ACTIVE is empty.
}
