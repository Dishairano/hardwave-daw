// Drawing for the built-in plug-in windows: the lineup's knob and every
// display. A display draws either what the plug-in's parameters set (a
// filter's curve worked out with the plug-in's own formulas, a delay's
// echoes from its feedback) or what the slot measured (levels, gain
// reduction, the stereo field). Nothing here invents a reading.

import type { FamilyColours } from './pluginLayouts'
import { EQ_BAND_COLOURS, EQ_BAND_KINDS } from './pluginLayouts'

export interface PluginParam {
  id: number
  name: string
  defaultValue: number
  value: number
  min: number
  max: number
  unit: string
  automatable: boolean
  text?: string | null
  options?: string[] | null
  /** Built-ins: the text at 101 even steps from min to max. */
  texts?: string[] | null
}

/** What the slot measured, polled while the window is open. */
export interface LiveData {
  /** Peaks since the last poll, linear: in L, in R, out L, out R. */
  levels: [number, number, number, number]
  /** Gain reduction now, dB (0 or less), or null for a plug-in without. */
  gr: number | null
  /** The last 4 s of gain reduction and input peak (dB), oldest first. */
  grHistory: number[]
  inHistory: number[]
  /** The last output frames [l, r, l, r, ..], for a stereo field. */
  scope: number[] | null
  /** Wavetable frames for the current bank, when the window has them. */
  table: number[][] | null
}

export const NO_LIVE: LiveData = {
  levels: [0, 0, 0, 0], gr: null, grHistory: [], inHistory: [], scope: null, table: null,
}

export const toDb = (lin: number) => (lin > 1e-6 ? 20 * Math.log10(lin) : -120)

/**
 * A plug-in's parameters with the values the window shows, and their text.
 * `exact` holds the text the plug-in gave for a value it was set to; any
 * other value is named from the 101-step table.
 */
export class ParamView {
  constructor(
    readonly params: PluginParam[],
    readonly values: Record<number, number>,
    readonly exact: Record<number, string>,
  ) {}

  par(name: string): PluginParam | undefined {
    return this.params.find((p) => p.name === name)
  }

  norm(q: PluginParam, v = this.values[q.id] ?? q.value): number {
    const span = q.max - q.min
    return span ? (v - q.min) / span : 0
  }

  denorm(q: PluginParam, t: number): number {
    return q.min + Math.max(0, Math.min(1, t)) * (q.max - q.min)
  }

  /** A parameter's value in its own range. */
  v(name: string, d = 0): number {
    const q = this.par(name)
    return q ? (this.values[q.id] ?? q.value) : d
  }

  /** A parameter's position 0..1. */
  n(name: string, d = 0.5): number {
    const q = this.par(name)
    return q ? this.norm(q) : d
  }

  text(q: PluginParam, v = this.values[q.id] ?? q.value): string {
    if (v === this.values[q.id] && this.exact[q.id] != null) return this.exact[q.id]
    const t = this.norm(q, v)
    if (q.texts && q.texts.length === 101) return q.texts[Math.max(0, Math.min(100, Math.round(t * 100)))]
    if (q.options && q.options.length > 1) return q.options[Math.round(t * (q.options.length - 1))]
    const num = Math.abs(v) >= 100 ? v.toFixed(0) : v.toFixed(2)
    return q.unit ? `${num} ${q.unit}` : num
  }

  t(name: string): string {
    const q = this.par(name)
    return q ? this.text(q) : ''
  }

  /**
   * A value as a number in its unit, read from the plug-in's text, so a
   * parameter stored 0..1 still gives hertz, seconds, bits or decibels.
   * Seconds come back as milliseconds when the text says "s".
   */
  num(name: string, d = 0): number {
    const m = /(-?[\d.]+)\s*(k?)(Hz|s|ms)?/.exec(this.t(name))
    if (!m) return d
    let x = parseFloat(m[1]) * (m[2] === 'k' ? 1000 : 1)
    if (m[3] === 's') x *= 1000
    return Number.isFinite(x) ? x : d
  }

  /** The index of a parameter's choice. */
  choice(name: string): number {
    const q = this.par(name)
    if (!q?.options?.length) return 0
    return Math.round(this.norm(q) * (q.options.length - 1))
  }

  isSwitch(q: PluginParam): boolean {
    return q.unit === 'toggle' || (!!q.texts && q.texts[0] === 'Off' && q.texts[100] === 'On')
  }
}

// ------------------------------------------------------------------ knob

/** The lineup's knob (WbKnob): a 270° arc, a dark body and a glowing dot. */
export function drawKnob(c: HTMLCanvasElement, t: number, col: FamilyColours) {
  const g = c.getContext('2d')
  if (!g) return
  const w = c.width, h = c.height, cx = w / 2, cy = h / 2, s = w / 92
  const r = Math.min(w, h) / 2 - 3 * s
  g.clearRect(0, 0, w, h)
  const a0 = Math.PI * 0.75, sweep = Math.PI * 1.5, av = a0 + t * sweep
  g.lineCap = 'round'
  g.beginPath(); g.arc(cx, cy, r, a0, a0 + sweep); g.strokeStyle = '#252532'; g.lineWidth = 3 * s; g.stroke()
  if (t > 0.001) {
    const grd = g.createLinearGradient(cx - r, cy, cx + r, cy)
    grd.addColorStop(0, col.deep); grd.addColorStop(1, col.a2)
    g.beginPath(); g.arc(cx, cy, r, a0, av); g.strokeStyle = grd; g.stroke()
  }
  g.beginPath(); g.arc(cx, cy, r - 7 * s, 0, Math.PI * 2)
  const bg = g.createRadialGradient(cx - 2 * s, cy - 2 * s, 1, cx, cy, r - 7 * s)
  bg.addColorStop(0, '#1a1a24'); bg.addColorStop(1, '#0c0c12')
  g.fillStyle = bg; g.fill(); g.strokeStyle = '#2a2a38'; g.lineWidth = 1 * s; g.stroke()
  const pd = r - 10 * s
  g.beginPath(); g.arc(cx + Math.cos(av) * pd, cy + Math.sin(av) * pd, 2.5 * s, 0, Math.PI * 2)
  g.fillStyle = t > 0.001 ? col.a2 : '#8b8b99'
  if (t > 0.001) { g.shadowColor = col.a; g.shadowBlur = 6 * s }
  g.fill(); g.shadowBlur = 0
}

// ------------------------------------------------------------- filters

export type BiquadKind = 'lowpass' | 'highpass' | 'bandpass' | 'notch' | 'peak' | 'lowshelf' | 'highshelf'

/**
 * The magnitude in dB of the plug-ins' biquad (hardwave-dsp biquad.rs, the
 * same formulas) at `hz`, at 48 kHz.
 */
export function biquadDb(kind: BiquadKind, fc: number, q: number, gainDb: number, hz: number, sr = 48000): number {
  const cutoff = Math.max(1, Math.min(fc, sr * 0.499))
  const qq = Math.max(0.0001, q)
  const w0 = (2 * Math.PI * cutoff) / sr
  const cw = Math.cos(w0), sw = Math.sin(w0)
  const alpha = sw / (2 * qq)
  const A = Math.pow(10, gainDb / 40)
  let b0 = 1, b1 = 0, b2 = 0, a0 = 1, a1 = 0, a2 = 0
  if (kind === 'lowpass') { b0 = (1 - cw) / 2; b1 = 1 - cw; b2 = b0; a0 = 1 + alpha; a1 = -2 * cw; a2 = 1 - alpha }
  else if (kind === 'highpass') { b0 = (1 + cw) / 2; b1 = -(1 + cw); b2 = b0; a0 = 1 + alpha; a1 = -2 * cw; a2 = 1 - alpha }
  else if (kind === 'bandpass') { b0 = alpha; b1 = 0; b2 = -alpha; a0 = 1 + alpha; a1 = -2 * cw; a2 = 1 - alpha }
  else if (kind === 'notch') { b0 = 1; b1 = -2 * cw; b2 = 1; a0 = 1 + alpha; a1 = -2 * cw; a2 = 1 - alpha }
  else if (kind === 'peak') { b0 = 1 + alpha * A; b1 = -2 * cw; b2 = 1 - alpha * A; a0 = 1 + alpha / A; a1 = -2 * cw; a2 = 1 - alpha / A }
  else {
    // Shelf slope S = 1, as the plug-in sets it.
    const sa = (sw / 2) * Math.SQRT2
    const rA = 2 * Math.sqrt(A) * sa
    if (kind === 'lowshelf') {
      b0 = A * ((A + 1) - (A - 1) * cw + rA); b1 = 2 * A * ((A - 1) - (A + 1) * cw); b2 = A * ((A + 1) - (A - 1) * cw - rA)
      a0 = (A + 1) + (A - 1) * cw + rA; a1 = -2 * ((A - 1) + (A + 1) * cw); a2 = (A + 1) + (A - 1) * cw - rA
    } else {
      b0 = A * ((A + 1) + (A - 1) * cw + rA); b1 = -2 * A * ((A - 1) + (A + 1) * cw); b2 = A * ((A + 1) + (A - 1) * cw - rA)
      a0 = (A + 1) - (A - 1) * cw + rA; a1 = 2 * ((A - 1) - (A + 1) * cw); a2 = (A + 1) - (A - 1) * cw - rA
    }
  }
  const w = (2 * Math.PI * hz) / sr
  const c1 = Math.cos(w), s1 = Math.sin(w), c2 = Math.cos(2 * w), s2 = Math.sin(2 * w)
  const nr = b0 + b1 * c1 + b2 * c2, ni = -(b1 * s1 + b2 * s2)
  const dr = a0 + a1 * c1 + a2 * c2, di = -(a1 * s1 + a2 * s2)
  const mag = Math.sqrt((nr * nr + ni * ni) / Math.max(1e-30, dr * dr + di * di))
  return 20 * Math.log10(Math.max(1e-9, mag))
}

/** The EQ's bands as the plug-in has them. */
export interface EqBand { on: boolean; f: number; g: number; q: number; kind: BiquadKind }

export function eqBands(v: ParamView): EqBand[] {
  const n = v.params.filter((x) => /^Band \d+ Enabled$/.test(x.name)).length
  return Array.from({ length: n }, (_, i) => ({
    on: v.n(`Band ${i + 1} Enabled`, 0) >= 0.5,
    f: v.v(`Band ${i + 1} Frequency`, 1000),
    g: v.v(`Band ${i + 1} Gain`, 0),
    q: v.v(`Band ${i + 1} Q`, 0.7),
    kind: (EQ_BAND_KINDS[i] ?? 'peak') as BiquadKind,
  }))
}

/** Frequency to x on a 20 Hz..20 kHz log axis, and back. */
export const freqX = (hz: number, W: number) => (Math.log10(Math.max(20, hz) / 20) / 3) * W
export const xFreq = (x: number, W: number) => 20 * Math.pow(1000, Math.max(0, Math.min(1, x / W)))
/** The EQ display's gain scale: ±24 dB over the height, less a margin. */
export const EQ_RANGE_DB = 24
export const eqY = (db: number, H: number) => H / 2 - (db / EQ_RANGE_DB) * (H / 2 - 10)
export const yEq = (y: number, H: number) => ((H / 2 - y) / (H / 2 - 10)) * EQ_RANGE_DB

// ------------------------------------------------------------ displays

function setup(c: HTMLCanvasElement) {
  const dpr = window.devicePixelRatio || 1, W = c.clientWidth, H = c.clientHeight
  if (c.width !== Math.round(W * dpr) || c.height !== Math.round(H * dpr)) {
    c.width = Math.round(W * dpr); c.height = Math.round(H * dpr)
  }
  const g = c.getContext('2d')
  if (g) { g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, W, H) }
  return { g, W, H }
}

const rgb = (hex: string) => {
  const n = parseInt(hex.slice(1), 16)
  return `${n >> 16},${(n >> 8) & 255},${n & 255}`
}

/** Seeded noise, so a texture is the same on every frame. */
function seeded(seed: number) {
  let s = seed
  return () => (s = (s * 16807) % 2147483647) / 2147483647
}

const FREQS = [50, 100, 200, 500, 1000, 2000, 5000, 10000]

/** Kinds whose drawing follows the slot's measurements, redrawn per poll. */
export const LIVE_KINDS = new Set(['transfer', 'grhistory', 'gate', 'gonio', 'lr', 'reels'])
/** Kinds that need the slot's output frames. */
export const SCOPE_KINDS = new Set(['gonio'])

/**
 * Draw one display. Returns the text for its top-right corner.
 */
export function drawDisplay(
  c: HTMLCanvasElement, kind: string, v: ParamView, f: FamilyColours, live: LiveData,
  opts: { eqBand?: number; time?: number } = {},
): string {
  const { g, W, H } = setup(c)
  if (!g || W === 0 || H === 0) return ''
  const grid = (xs: number[], ys: number[]) => {
    g.strokeStyle = 'rgba(255,255,255,0.045)'; g.lineWidth = 1
    xs.forEach((x) => { g.beginPath(); g.moveTo(Math.floor(x) + 0.5, 0); g.lineTo(Math.floor(x) + 0.5, H); g.stroke() })
    ys.forEach((y) => { g.beginPath(); g.moveTo(0, Math.floor(y) + 0.5); g.lineTo(W, Math.floor(y) + 0.5); g.stroke() })
  }
  const area = (pts: [number, number][], col = f.a, top = 0.28, base = H) => {
    if (!pts.length) return
    const gr = g.createLinearGradient(0, 0, 0, H)
    gr.addColorStop(0, `rgba(${rgb(col)},${top})`); gr.addColorStop(1, `rgba(${rgb(col)},0)`)
    g.beginPath(); g.moveTo(pts[0][0], base); pts.forEach(([x, y]) => g.lineTo(x, y)); g.lineTo(pts[pts.length - 1][0], base); g.closePath()
    g.fillStyle = gr; g.fill()
  }
  const line = (pts: [number, number][], col = f.a2, w = 2, glow = true) => {
    g.beginPath(); pts.forEach(([x, y], i) => (i ? g.lineTo(x, y) : g.moveTo(x, y)))
    g.strokeStyle = col; g.lineWidth = w
    if (glow) { g.shadowColor = col; g.shadowBlur = 8 }
    g.stroke(); g.shadowBlur = 0
  }
  const lab = (t: string, x: number, y: number, col = '#4a4a58', al: CanvasTextAlign = 'left') => {
    g.fillStyle = col; g.font = '500 9px "JetBrains Mono", ui-monospace, monospace'; g.textAlign = al; g.fillText(t, x, y)
  }
  const fx = (hz: number) => freqX(hz, W)
  const freqLabels = () => ([[100, '100'], [1000, '1k'], [10000, '10k']] as const).forEach(([hz, l]) => lab(l, fx(hz) + 4, H - 8))
  const dot = (x: number, y: number, r = 5, col = '#fff') => { g.beginPath(); g.arc(x, y, r, 0, Math.PI * 2); g.fillStyle = col; g.fill() }

  if (kind === 'eq') {
    grid(FREQS.map(fx), [H * 0.25, H * 0.5, H * 0.75])
    freqLabels()
    ;['+12', '0', '-12'].forEach((l, i) => lab(l, W - 6, eqY(12 - i * 12, H) - 4, '#4a4a58', 'right'))
    const bands = eqBands(v)
    const out = v.v('Output Gain', 0)
    const resp = (hz: number) => bands.reduce((a, b) => a + (b.on ? biquadDb(b.kind, b.f, b.q, b.g, hz) : 0), 0) + out
    const pts: [number, number][] = []
    for (let x = 0; x <= W; x += 2) pts.push([x, eqY(resp(xFreq(x, W)), H)])
    area(pts, '#2DD4BF', 0.22, H / 2); line(pts, '#2DD4BF', 2.2)
    const sel = opts.eqBand ?? 0
    bands.forEach((b, i) => {
      if (!b.on && i !== sel) return
      const col = EQ_BAND_COLOURS[i] ?? '#fff'
      const x = fx(b.f), y = eqY(b.g, H)
      if (i === sel) { g.beginPath(); g.arc(x, y, 12, 0, Math.PI * 2); g.strokeStyle = col; g.lineWidth = 1.5; g.stroke() }
      g.beginPath(); g.arc(x, y, 7, 0, Math.PI * 2); g.fillStyle = b.on ? col : '#3a3a48'
      g.shadowColor = col; g.shadowBlur = b.on ? 10 : 0; g.fill(); g.shadowBlur = 0
      lab(String(i + 1), x, y + 3, '#060608', 'center')
    })
    return `BAND ${sel + 1} · ${v.t(`Band ${sel + 1} Frequency`)} · ${v.t(`Band ${sel + 1} Gain`)}`
  }

  if (kind === 'transfer') {
    const S = Math.min(H - 24, W - 30), X0 = (W - S) / 2, Y0 = 14
    g.strokeStyle = 'rgba(255,255,255,.06)'; g.strokeRect(X0 + 0.5, Y0 + 0.5, S, S)
    for (let i = 1; i < 6; i++) { const d = (S * i) / 6; g.beginPath(); g.moveTo(X0 + d, Y0); g.lineTo(X0 + d, Y0 + S); g.moveTo(X0, Y0 + d); g.lineTo(X0 + S, Y0 + d); g.stroke() }
    line([[X0, Y0 + S], [X0 + S, Y0]], 'rgba(255,255,255,.12)', 1, false)
    const thr = v.v('Threshold', -20), ratio = Math.max(1, v.v('Ratio', 2)), knee = Math.max(0, v.v('Knee', 0))
    const out = (x: number) => {
      const d = x - thr
      if (2 * d < -knee) return x
      if (knee > 0 && 2 * Math.abs(d) <= knee) return x + ((1 / ratio - 1) * Math.pow(d + knee / 2, 2)) / (2 * knee)
      return thr + d / ratio
    }
    const pts: [number, number][] = []
    for (let i = 0; i <= 120; i++) { const x = -60 + i / 2; pts.push([X0 + ((x + 60) / 60) * S, Y0 + S - ((out(x) + 60) / 60) * S]) }
    area(pts, f.a, 0.16); line(pts, f.a2, 2.4)
    const tx = X0 + ((thr + 60) / 60) * S
    g.setLineDash([3, 4]); line([[tx, Y0], [tx, Y0 + S]], `rgba(${rgb(f.a2)},.5)`, 1, false); g.setLineDash([])
    // Where the input is now, on the curve.
    const inDb = toDb(Math.max(live.levels[0], live.levels[1]))
    if (inDb > -60) dot(X0 + ((inDb + 60) / 60) * S, Y0 + S - ((out(Math.min(0, inDb)) + 60) / 60) * S)
    lab('-60', X0, Y0 + S + 10); lab('0 dB', X0 + S, Y0 + S + 10, '#4a4a58', 'right')
    return `${v.t('Threshold')} · ${v.t('Ratio')}`
  }

  if (kind === 'grhistory' || kind === 'gate') {
    grid([W / 4, W / 2, (W * 3) / 4], [H / 3, (H * 2) / 3])
    const n = Math.max(2, live.grHistory.length)
    const xAt = (i: number) => (i / (n - 1)) * W
    // The input, grey, -60..0 dB from the bottom.
    const ins: [number, number][] = live.inHistory.map((db, i) => [xAt(i), H - 14 - (Math.max(0, db + 60) / 60) * (H - 34)])
    area(ins, '#8b8b99', 0.14); line(ins, '#6b6b78', 1.2, false)
    if (kind === 'gate') {
      const open: [number, number][] = live.grHistory.map((gr, i) => [xAt(i), gr > -1 ? 14 : H - 14])
      line(open, f.a2, 2)
      const now = live.grHistory[live.grHistory.length - 1] ?? 0
      return live.gr == null ? '' : now > -1 ? 'OPEN' : 'CLOSED'
    }
    // Gain reduction from the top, 0..24 dB.
    const red: [number, number][] = live.grHistory.map((gr, i) => [xAt(i), 14 + (Math.min(24, -gr) / 24) * (H - 34)])
    line(red, f.a2, 2)
    lab('0', 6, 22); lab('-24 dB', 6, H - 18)
    const max = live.grHistory.reduce((m, x) => Math.min(m, x), 0)
    return `MAX ${max.toFixed(1)} dB`
  }

  if (kind === 'transient') {
    grid([W / 4, W / 2, (W * 3) / 4], [H / 2])
    const a = v.num('Attack'), s = v.num('Sustain')
    const pts: [number, number][] = [], o: [number, number][] = []
    for (let x = 0; x <= W; x += 2) {
      const t = ((x / W) * 4) % 1
      const e = t < 0.04 ? t / 0.04 : Math.exp(-(t - 0.04) * 6)
      o.push([x, H - 16 - e * (H - 40) * 0.7])
      const m = t < 0.1 ? Math.pow(10, a / 20) : Math.pow(10, s / 20)
      pts.push([x, H - 16 - Math.min(1, e * 0.7 * m) * (H - 40)])
    }
    line(o, 'rgba(255,255,255,.15)', 1.2, false); area(pts, f.a, 0.2); line(pts, f.a2, 2)
    return `${v.t('Attack')} · ${v.t('Sustain')}`
  }

  if (kind === 'clip' || kind === 'shaper') {
    const S = Math.min(H - 28, W - 40), X0 = (W - S) / 2, Y0 = 14
    g.strokeStyle = 'rgba(255,255,255,.06)'; g.strokeRect(X0 + 0.5, Y0 + 0.5, S, S)
    line([[X0, Y0 + S], [X0 + S, Y0]], 'rgba(255,255,255,.12)', 1, false)
    line([[X0, Y0 + S / 2], [X0 + S, Y0 + S / 2]], 'rgba(255,255,255,.05)', 1, false)
    line([[X0 + S / 2, Y0], [X0 + S / 2, Y0 + S]], 'rgba(255,255,255,.05)', 1, false)
    let fn: (x: number) => number
    let label: string
    if (kind === 'clip') {
      const drive = Math.pow(10, v.num('Drive') / 20), ceil = Math.pow(10, v.num('Ceiling') / 20)
      fn = (x) => Math.max(-ceil, Math.min(ceil, x * drive))
      label = `${v.t('Drive')} · CEILING ${v.t('Ceiling')}`
    } else {
      const drive = v.par('Drive') ? Math.pow(10, v.num('Drive') / 20) : 1 + 9 * v.n('Amount', 0.4)
      const mode = v.t('Mode')
      fn = mode === 'Hard' ? (x) => Math.max(-1, Math.min(1, x * drive))
        : mode === 'Bitcrush' ? (x) => Math.round(Math.tanh(x * drive) * 6) / 6
          : (x) => Math.tanh(x * drive) / Math.tanh(Math.max(1, drive))
      label = `DRIVE ${drive.toFixed(1)}x`
    }
    const pts: [number, number][] = []
    for (let i = 0; i <= 200; i++) { const x = -1 + i / 100; pts.push([X0 + ((x + 1) / 2) * S, Y0 + (1 - (Math.max(-1, Math.min(1, fn(x))) + 1) / 2) * S]) }
    line(pts, f.a2, 2.6)
    return label
  }

  if (kind === 'xover') {
    const lo = fx(v.num('Low/Mid', 200)), hi = fx(v.num('Mid/High', 2000))
    const cols = ['#F59E0B', '#FBBF24', '#FB923C']
    ;([[0, lo], [lo, hi], [hi, W]] as const).forEach(([a, b], i) => {
      g.fillStyle = `rgba(${rgb(cols[i])},.08)`; g.fillRect(a, 0, b - a, H)
      lab(['LOW', 'MID', 'HIGH'][i], (a + b) / 2, H - 10, '#8b8b99', 'center')
    })
    grid(FREQS.map(fx), [])
    ;[lo, hi].forEach((x) => { line([[x, 0], [x, H]], f.a2, 2); dot(x, H / 2, 6, f.a2) })
    return `${v.t('Low/Mid')} · ${v.t('Mid/High')}`
  }

  if (kind === 'filter') {
    grid(FREQS.map(fx), [eqY(0, H)])
    const auto = !v.par('Cutoff')
    const fc = auto ? v.num('Base', 200) : v.num('Cutoff', 1000)
    const q = auto ? v.num('Resonance', 1) : v.num('Q', 0.7)
    const modeName = auto ? 'Low pass' : v.t('Mode')
    const kindOf: Record<string, BiquadKind> = { 'Low pass': 'lowpass', 'High pass': 'highpass', 'Band pass': 'bandpass', Notch: 'notch' }
    const k = kindOf[modeName] ?? 'lowpass'
    if (auto) {
      // How far the envelope can open it.
      const top = fc * Math.pow(2, v.num('Range', 0))
      g.fillStyle = `rgba(${rgb(f.a)},.07)`; g.fillRect(fx(fc), 0, fx(top) - fx(fc), H)
      line([[fx(top), 0], [fx(top), H]], `rgba(${rgb(f.a2)},.4)`, 1, false)
    }
    const pts: [number, number][] = []
    for (let x = 0; x <= W; x += 2) pts.push([x, eqY(Math.max(-EQ_RANGE_DB, biquadDb(k, fc, q, 0, xFreq(x, W))), H)])
    area(pts, f.a, 0.22); line(pts, f.a2, 2.4)
    dot(fx(fc), eqY(Math.max(-EQ_RANGE_DB, biquadDb(k, fc, q, 0, fc)), H), 6)
    freqLabels()
    return (auto ? v.t('Base') : v.t('Cutoff')).toUpperCase()
  }

  if (kind === 'follow') {
    const atk = Math.max(0.5, v.num('Attack', 5)), rel = Math.max(5, v.num('Release', 100))
    const span = 2 * (atk + rel)
    const pts: [number, number][] = []
    for (let x = 0; x <= W; x += 2) {
      const ms = (x / W) * span
      const e = ms < atk ? ms / atk : Math.exp(-(ms - atk) / (rel / 3))
      pts.push([x, H - 10 - e * (H - 24)])
    }
    area(pts, f.a, 0.2); line(pts, f.a2, 1.8)
    return `${Math.round(span)} ms`
  }

  if (kind === 'taps') {
    grid([W / 4, W / 2, (W * 3) / 4], [H / 2])
    const fb = Math.min(1, v.num('Feedback', 35) / 100), pp = v.n('Ping-Pong', 0) >= 0.5
    let a = 1
    for (let i = 0; i < 14 && a > 0.001; i++) {
      const x = 30 + (i * (W - 60)) / 13, hh = (H - 46) * a
      const col = pp && i % 2 ? '#38BDF8' : f.a2
      g.fillStyle = `rgba(${rgb(col)},${0.25 + 0.75 * a})`; g.shadowColor = col; g.shadowBlur = 8
      g.fillRect(x - 3, H - 20 - hh, 6, Math.max(1, hh)); g.shadowBlur = 0
      a *= fb
    }
    return (pp ? 'PING-PONG · ' : 'MONO · ') + v.t('Time')
  }

  if (kind === 'decay' || kind === 'ir') {
    const rnd = seeded(kind === 'ir' ? 11 + v.choice('Preset') : 5)
    const pre = v.num('Pre-Delay', 0)
    if (kind === 'decay') {
      const rt = Math.max(100, v.num('Decay', 2000))
      const span = pre + rt * 1.1
      grid([W / 4, W / 2, (W * 3) / 4], [])
      const top: [number, number][] = [], bot: [number, number][] = []
      for (let x = 0; x <= W; x++) {
        const ms = (x / W) * span
        const env = ms < pre ? 0 : Math.pow(10, (-3 * (ms - pre)) / rt)
        const n = 0.6 + 0.4 * rnd()
        top.push([x, H / 2 - env * n * (H / 2 - 12)]); bot.push([x, H / 2 + env * n * (H / 2 - 12)])
      }
      line(top, f.a2, 1, false); line(bot, f.a2, 1, false)
      lab('0', 4, H - 6); lab(span >= 1000 ? `${(span / 1000).toFixed(1)} s` : `${Math.round(span)} ms`, W - 4, H - 6, '#4a4a58', 'right')
      return `RT60 ${v.t('Decay')}`
    }
    // The impulse with the part the Tail Length keeps lit and the rest dim.
    const keep = Math.max(0.1, Math.min(1, v.num('Tail Length', 100) / 100))
    grid([W / 4, W / 2, (W * 3) / 4], [])
    const s0 = Math.min(0.3, pre / 1000)
    for (let x = 0; x <= W; x++) {
      const t = x / W
      const env = t < s0 ? 0 : Math.exp(-(t - s0) * 4.5)
      const a = env * (0.5 + 0.5 * rnd()) * (H / 2 - 12)
      g.fillStyle = t <= keep ? `rgba(${rgb(f.a2)},.85)` : 'rgba(255,255,255,.1)'
      g.fillRect(x, H / 2 - a, 1, a * 2)
    }
    line([[keep * W, 8], [keep * W, H - 8]], 'rgba(255,255,255,.35)', 1, false)
    return `${v.t('Preset').toUpperCase()} · ${v.t('Tail Length')}`
  }

  if (kind === 'lfo' || kind === 'lfo2' || kind === 'pan') {
    grid([W / 4, W / 2, (W * 3) / 4], [H / 2])
    const rate = Math.max(0, v.num('Rate', 1)) // Hz: cycles over the 1 s shown
    const depth = Math.max(0.05, Math.min(1, v.n('Depth', 0.6)))
    const sh = v.t('Shape') || 'Sine'
    const wave = (ph: number) => {
      const p = ((ph / (2 * Math.PI)) % 1 + 1) % 1
      return sh === 'Triangle' ? 1 - 4 * Math.abs(p - 0.5) : sh === 'Square' ? (p < 0.5 ? 1 : -1) : Math.sin(ph)
    }
    const mk = (off: number): [number, number][] => {
      const pts: [number, number][] = []
      for (let x = 0; x <= W; x += 2) pts.push([x, H / 2 - wave((x / W) * rate * 2 * Math.PI + off) * depth * (H / 2 - 16)])
      return pts
    }
    if (kind === 'pan') { lab('L', 8, 14); lab('R', 8, H - 8) }
    line(mk(0), f.a2, 2)
    // The second channel: tremolo's Stereo offset, chorus's width (its
    // right LFO runs up to half a cycle behind).
    if (kind === 'lfo2') line(mk(v.n('Width', 1) * Math.PI), '#38BDF8', 1.6)
    if (v.par('Stereo')) line(mk((v.num('Stereo', 0) / 360) * 2 * Math.PI), '#38BDF8', 1.6)
    return 'RATE ' + v.t('Rate')
  }

  if (kind === 'notches') {
    grid(FREQS.map(fx), [])
    const base = v.num('Base', 500), spread = v.num('Spread', 1), depth = v.num('Depth', 1)
    // Six all-pass stages give three notches, `spread` octaves apart.
    const notches = [0, 1, 2].map((k) => base * Math.pow(2, k * spread))
    notches.forEach((hz) => {
      const lo = fx(hz / Math.pow(2, depth / 2)), hi = fx(hz * Math.pow(2, depth / 2))
      g.fillStyle = `rgba(${rgb(f.a)},.08)`; g.fillRect(lo, 0, hi - lo, H)
    })
    const pts: [number, number][] = []
    for (let x = 0; x <= W; x += 2) {
      const hz = xFreq(x, W)
      let d = 0
      notches.forEach((n) => { d += Math.exp(-Math.pow(Math.log2(hz / n) * 6, 2)) })
      pts.push([x, 16 + Math.min(1, d) * (H - 36)])
    }
    area(pts, f.a, 0.18, 0); line(pts, f.a2, 2)
    freqLabels()
    return `${v.t('Base')} · ${v.t('Spread')}`
  }

  if (kind === 'comb') {
    grid(FREQS.map(fx), [])
    const tau = Math.max(0.05, v.num('Depth', 2)) / 1000
    const gfb = Math.min(0.95, v.num('Feedback', 50) / 100) * (v.n('Invert', 0) >= 0.5 ? -1 : 1)
    const pts: [number, number][] = []
    for (let x = 0; x <= W; x += 1) {
      const w = 2 * Math.PI * xFreq(x, W) * tau
      // Feedback comb: 1 / |1 - g e^-jwt|, in dB, -12..+18.
      const den = Math.sqrt(Math.pow(1 - gfb * Math.cos(w), 2) + Math.pow(gfb * Math.sin(w), 2))
      const db = 20 * Math.log10(1 / Math.max(1e-6, den))
      pts.push([x, H / 2 - (Math.max(-12, Math.min(18, db)) / 18) * (H / 2 - 14)])
    }
    area(pts, f.a, 0.18); line(pts, f.a2, 1.6)
    freqLabels()
    return `${v.t('Depth')} · ${v.t('Feedback')}`
  }

  if (kind === 'ring') {
    grid(FREQS.map(fx), [])
    const fc = v.num('Frequency', 200)
    ;([[440, 0.8, '#8b8b99'], [440 + fc, 0.6, f.a2], [Math.max(20, Math.abs(440 - fc)), 0.6, f.a2]] as const).forEach(([hz, h, col]) => {
      const x = fx(hz)
      g.fillStyle = col; g.shadowColor = col; g.shadowBlur = 8
      g.fillRect(x - 2, H - 16 - h * (H - 40), 4, h * (H - 40)); g.shadowBlur = 0
    })
    lab('in', fx(440) + 6, 26, '#8b8b99')
    freqLabels()
    return `${Math.round(Math.abs(440 - fc))} + ${Math.round(440 + fc)} Hz`
  }

  if (kind === 'reels') {
    const moving = Math.max(live.levels[2], live.levels[3]) > 1e-4
    const t0 = moving ? (opts.time ?? 0) : 0
    ;[W * 0.3, W * 0.7].forEach((cx) => {
      const cy = H / 2, r = Math.min(W * 0.17, H / 2 - 16)
      g.beginPath(); g.arc(cx, cy, r, 0, Math.PI * 2); g.strokeStyle = '#2a2a38'; g.lineWidth = 2; g.stroke()
      g.beginPath(); g.arc(cx, cy, r * 0.3, 0, Math.PI * 2); g.stroke()
      for (let k = 0; k < 3; k++) {
        const a = t0 / 600 + k * 2.09
        g.beginPath(); g.moveTo(cx + Math.cos(a) * r * 0.32, cy + Math.sin(a) * r * 0.32)
        g.lineTo(cx + Math.cos(a) * r * 0.95, cy + Math.sin(a) * r * 0.95)
        g.strokeStyle = f.a2; g.lineWidth = 2; g.stroke()
      }
    })
    line([[W * 0.3, H - 14], [W * 0.7, H - 14]], f.a, 2)
    return moving ? 'RUNNING' : 'NO SIGNAL'
  }

  if (kind === 'steps') {
    grid([], [H / 2])
    const bits = Math.max(1, Math.round(v.num('Bits', 8)))
    const levels = Math.pow(2, Math.min(bits, 8)) / 2
    const factor = Math.max(1, v.num('Rate', 1))
    // Two cycles across the width, sampled every `factor` of 256 steps.
    const hold = Math.max(1, (W / 256) * factor)
    const o: [number, number][] = [], pts: [number, number][] = []
    for (let x = 0; x <= W; x += 1) o.push([x, H / 2 - Math.sin((x / W) * Math.PI * 4) * (H / 2 - 18)])
    for (let x = 0; x <= W; x += hold) {
      const s = Math.round(Math.sin((x / W) * Math.PI * 4) * levels) / levels
      pts.push([x, H / 2 - s * (H / 2 - 18)]); pts.push([x + hold, H / 2 - s * (H / 2 - 18)])
    }
    line(o, 'rgba(255,255,255,.12)', 1, false); line(pts, f.a2, 2)
    return `${v.t('Bits')} · ${v.t('Rate')}`
  }

  if (kind === 'excite' || kind === 'sub' || kind === 'noise') {
    grid(FREQS.map(fx), [])
    const yDb = (db: number) => H / 2 - (db / 30) * (H / 2 - 14)
    const pts: [number, number][] = []
    for (let x = 0; x <= W; x += 2) {
      const hz = xFreq(x, W)
      let db = 0
      if (kind === 'noise') {
        // White is flat, pink falls 3 dB an octave, brown 6.
        const slope = [0, -3, -6][v.choice('Colour')] ?? 0
        db = 12 + slope * Math.log2(hz / 20)
      } else if (kind === 'excite') {
        const fc = v.num('Frequency', 4000)
        db = hz > fc ? v.n('Amount', 0.4) * 12 * Math.min(1, Math.log2(hz / fc)) : 0
      } else {
        const fc = v.num('Crossover', 80)
        db = hz < fc ? v.num('Boost', 0) : v.num('Boost', 0) * Math.max(0, 1 - Math.log2(hz / fc) * 2)
      }
      pts.push([x, yDb(Math.max(-30, Math.min(30, db)))])
    }
    if (kind !== 'noise') line([[0, yDb(0)], [W, yDb(0)]], 'rgba(255,255,255,.14)', 1, false)
    area(pts, f.a, 0.22); line(pts, f.a2, 2)
    freqLabels()
    return kind === 'noise' ? v.t('Colour').toUpperCase() : kind === 'excite' ? `ABOVE ${v.t('Frequency')}` : `${v.t('Boost')} BELOW ${v.t('Crossover')}`
  }

  if (kind === 'gonio') {
    const cx = W / 2, cy = H / 2, r = Math.min(H / 2 - 14, W / 2 - 20)
    g.beginPath(); g.arc(cx, cy, r, 0, Math.PI * 2); g.strokeStyle = `rgba(${rgb(f.a2)},.18)`; g.lineWidth = 1; g.stroke()
    ;([[0, -1], [1, 0], [0.7, -0.7], [-0.7, -0.7]] as const).forEach(([a, b]) => {
      g.beginPath(); g.moveTo(cx - a * r, cy - b * r); g.lineTo(cx + a * r, cy + b * r); g.strokeStyle = 'rgba(255,255,255,.06)'; g.stroke()
    })
    lab('MONO', cx, cy - r - 4, '#8b8b99', 'center'); lab('L', cx - r - 10, cy + 3); lab('R', cx + r + 4, cy + 3)
    const s = live.scope
    if (!s || s.length < 4) return 'NO SIGNAL'
    let peak = 0
    for (let i = 0; i < s.length; i++) peak = Math.max(peak, Math.abs(s[i]))
    if (peak < 1e-4) return 'NO SIGNAL'
    const k = (r * 0.9) / Math.max(0.25, peak * Math.SQRT2)
    g.fillStyle = `rgba(${rgb(f.a2)},.55)`
    for (let i = 0; i + 1 < s.length; i += 2) {
      const l = s[i], rr = s[i + 1]
      g.fillRect(cx + (rr - l) * k - 0.8, cy - (l + rr) * k - 0.8, 1.6, 1.6)
    }
    return `CORR ${correlation(s).toFixed(2)}`
  }

  if (kind === 'lr') {
    const bars = [toDb(live.levels[2]), toDb(live.levels[3])]
    ;[0, 1].forEach((ch) => {
      const y = 34 + ch * 52
      g.fillStyle = '#121219'; g.fillRect(28, y, W - 56, 20)
      const lv = Math.max(0, Math.min(1, 1 + bars[ch] / 60))
      const gr = g.createLinearGradient(28, 0, W - 28, 0)
      gr.addColorStop(0, f.deep); gr.addColorStop(0.7, f.a2); gr.addColorStop(0.9, '#FBBF24'); gr.addColorStop(1, '#EF4444')
      g.fillStyle = gr; g.fillRect(28, y, (W - 56) * lv, 20)
      lab(ch ? 'R' : 'L', 12, y + 14, '#8b8b99')
    })
    ;[-48, -24, -12, -6, 0].forEach((d) => lab(String(d), 28 + (W - 56) * (1 + d / 60), H - 14, '#4a4a58', 'center'))
    const pk = Math.max(bars[0], bars[1])
    return pk > -100 ? `${pk.toFixed(1)} dB` : '-inf dB'
  }

  if (kind.startsWith('wave:')) {
    const sh = v.t(kind.slice(5))
    const rnd = seeded(3)
    const pts: [number, number][] = []
    for (let x = 0; x <= W; x += 1) {
      const p = ((x / W) * 2) % 1
      const y = sh === 'Saw' ? p * 2 - 1 : sh === 'Square' ? (p < 0.5 ? 1 : -1) : sh === 'Triangle' ? 1 - 4 * Math.abs(p - 0.5) : sh === 'Noise' ? rnd() * 2 - 1 : Math.sin(p * 2 * Math.PI)
      pts.push([x, H / 2 - y * (H / 2 - 12)])
    }
    line(pts, f.a2, 1.8)
    return sh.toUpperCase()
  }

  if (kind === 'adsr' || kind === 'ar') {
    const A = v.n('Attack', 0.1), D = kind === 'ar' ? 0 : v.n('Decay', 0.3)
    const S = kind === 'ar' ? 1 : v.n('Sustain', 0.6), R = v.n('Release', 0.3)
    const seg = W - 40, x0 = 20, y0 = H - 14, top = 18
    const wd = (t: number, s: number) => (0.06 + 0.94 * t) * seg * s
    const xa = x0 + wd(A, 0.22), xd = kind === 'ar' ? xa : xa + wd(D, 0.22), xs = xd + seg * 0.18, xr = xs + wd(R, 0.3)
    const ys = y0 - S * (y0 - top)
    grid([xa, xd, xs], [])
    const pts: [number, number][] = [[x0, y0], [xa, top], [xd, ys], [xs, ys], [xr, y0]]
    area(pts, f.a, 0.22); line(pts, f.a2, 2.4)
    pts.slice(1, 4).forEach(([x, y]) => dot(x, y, 4.5))
    return kind === 'ar' ? `${v.t('Attack')} · ${v.t('Release')}` : `A ${v.t('Attack')} · R ${v.t('Release')}`
  }

  if (kind === 'algo') {
    // The eight routings of hardwave-dsp fm_synth.rs: modulators above,
    // carriers (what is heard) on the bottom row.
    const algo = v.choice('Algorithm')
    const A: { pos: [number, number][]; links: [number, number][]; carriers: number[] }[] = [
      { pos: [[0.5, 0.14], [0.5, 0.38], [0.5, 0.62], [0.5, 0.86]], links: [[0, 1], [1, 2], [2, 3]], carriers: [3] },
      { pos: [[0.5, 0.18], [0.32, 0.5], [0.68, 0.5], [0.5, 0.84]], links: [[0, 1], [0, 2], [1, 3], [2, 3]], carriers: [3] },
      { pos: [[0.25, 0.25], [0.5, 0.25], [0.75, 0.25], [0.5, 0.78]], links: [[0, 3], [1, 3], [2, 3]], carriers: [3] },
      { pos: [[0.32, 0.25], [0.32, 0.78], [0.68, 0.25], [0.68, 0.78]], links: [[0, 1], [2, 3]], carriers: [1, 3] },
      { pos: [[0.17, 0.62], [0.39, 0.62], [0.61, 0.62], [0.83, 0.62]], links: [], carriers: [0, 1, 2, 3] },
      { pos: [[0.25, 0.25], [0.25, 0.78], [0.5, 0.78], [0.75, 0.78]], links: [[0, 1]], carriers: [1, 2, 3] },
      { pos: [[0.5, 0.25], [0.25, 0.78], [0.5, 0.78], [0.75, 0.78]], links: [[0, 1], [0, 2], [0, 3]], carriers: [1, 2, 3] },
      { pos: [[0.32, 0.14], [0.32, 0.46], [0.32, 0.8], [0.68, 0.8]], links: [[0, 1], [1, 2]], carriers: [2, 3] },
    ]
    const a = A[Math.max(0, Math.min(7, algo))]
    a.links.forEach(([p, q]) => line([[a.pos[p][0] * W, a.pos[p][1] * H], [a.pos[q][0] * W, a.pos[q][1] * H]], 'rgba(255,255,255,.25)', 1.5, false))
    a.pos.forEach(([x, y], i) => {
      const carrier = a.carriers.includes(i)
      g.fillStyle = carrier ? `rgba(${rgb(f.a)},.18)` : '#121219'; g.strokeStyle = f.a2; g.lineWidth = 1.5
      g.beginPath(); g.roundRect(x * W - 22, y * H - 13, 44, 26, 6); g.fill(); g.stroke()
      lab('OP' + (i + 1), x * W, y * H + 3, f.a2, 'center')
    })
    return v.t('Algorithm').toUpperCase()
  }

  if (kind === 'table') {
    const frames = live.table
    const pos = v.n('Position', 0)
    if (!frames || !frames.length) return v.t('Bank').toUpperCase()
    const n = frames.length
    const sel = Math.round(pos * (n - 1))
    for (let k = 0; k < n; k++) {
      const y0 = H - 22 - (k * (H - 54)) / Math.max(1, n - 1), xo = k * 8
      const fr = frames[k]
      const pts: [number, number][] = fr.map((s, i) => [60 + (i / (fr.length - 1)) * (W - 140) + xo, y0 - s * 18])
      const on = k === sel
      line(pts, on ? f.a2 : `rgba(${rgb(f.a2)},${0.1 + 0.25 * (1 - Math.abs(k / Math.max(1, n - 1) - pos))})`, on ? 2.2 : 1, on)
    }
    return `${v.t('Bank').toUpperCase()} · ${v.t('Position')}`
  }

  if (kind === 'carrier') {
    grid([W / 4, W / 2, (W * 3) / 4], [H / 2])
    const which = v.t('Carrier')
    const rnd = seeded(9)
    const cycles = 4
    const pts: [number, number][] = []
    for (let x = 0; x <= W; x += 1) {
      const p = ((x / W) * cycles) % 1
      const y = which === 'Pulse' ? (p < 0.25 ? 1 : -1) : which === 'Noise' ? rnd() * 2 - 1 : p * 2 - 1
      pts.push([x, H / 2 - y * (H / 2 - 18)])
    }
    line(pts, f.a2, 1.6)
    return `${which.toUpperCase()} · ${v.t('Tone')}`
  }

  if (kind === 'slices') {
    const n = 16, rep = Math.max(1, Math.round(v.num('Repeats', 4))), decay = Math.min(1, v.num('Decay', 100) / 100)
    for (let i = 0; i < n; i++) {
      const k = i % 8
      const on = k < rep
      const x = 16 + (i * (W - 32)) / n
      g.fillStyle = on ? `rgba(${rgb(f.a2)},${Math.max(0.15, Math.pow(decay, k))})` : '#15151c'
      g.fillRect(x, 26, (W - 32) / n - 5, H - 52)
    }
    return 'SLICE ' + v.t('Slice')
  }

  return ''
}

/** Stereo correlation of frames [l, r, l, r, ..]: +1 mono, 0 wide, -1 out of phase. */
export function correlation(s: number[]): number {
  let lr = 0, ll = 0, rr = 0
  for (let i = 0; i + 1 < s.length; i += 2) { lr += s[i] * s[i + 1]; ll += s[i] * s[i]; rr += s[i + 1] * s[i + 1] }
  const d = Math.sqrt(ll * rr)
  return d > 1e-12 ? lr / d : 0
}

/**
 * The value that makes a parameter read `target` in its own unit, found in
 * its text table (so a parameter stored 0..1 can be set to 2 kHz).
 */
export function valueForNumber(view: ParamView, q: PluginParam, target: number): number {
  if (!q.texts || q.texts.length !== 101) return Math.max(q.min, Math.min(q.max, target))
  const nums = q.texts.map((t) => {
    const m = /(-?[\d.]+)\s*(k?)(Hz|s|ms)?/.exec(t)
    if (!m) return NaN
    let x = parseFloat(m[1]) * (m[2] === 'k' ? 1000 : 1)
    if (m[3] === 's') x *= 1000
    return x
  })
  let best = 0
  for (let i = 1; i < 101; i++) if (Math.abs(nums[i] - target) < Math.abs(nums[best] - target)) best = i
  // Between the two nearest steps, so a drag moves smoothly.
  const j = best < 100 && (nums[best + 1] - nums[best]) * (target - nums[best]) > 0 ? best + 1 : best > 0 ? best - 1 : best
  const a = nums[best], b = nums[j]
  const frac = j !== best && Number.isFinite(a) && Number.isFinite(b) && b !== a ? (target - a) / (b - a) : 0
  const t = (best + (j - best) * Math.max(0, Math.min(1, frac))) / 100
  return view.denorm(q, t)
}
