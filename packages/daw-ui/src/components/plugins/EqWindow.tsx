import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { PointerEvent as ReactPointerEvent, ReactNode, RefObject } from 'react'
import { biquadDb, type BiquadKind, type LiveData, type ParamView, type PluginParam } from './pluginDraw'
import './eqWindow.css'

/**
 * The EQ's window: the display is the instrument. Almost all of it is the
 * response over a live analyzer of the sound before and after the EQ; every
 * band is a point you pick up, and what belongs to a band opens beside it.
 * Approved as the direction for the built-ins on 10 Oct 2026 (suite:
 * daw-plugin-feel-mockup), after the founder found the card layout cheap.
 */

/** What the window needs from the frame around it (HwPluginWindow). */
export interface EqWindowProps {
  name: string
  view: ParamView
  ready: boolean
  error: string | null
  setParam: (q: PluginParam, v: number) => void
  openMenu: (e: React.MouseEvent, q: PluginParam) => void
  automated: Set<number>
  bus: { subscribe: (fn: () => void) => () => void }
  live: LiveData
  sampleRate: number
  ab: 'A' | 'B'
  onAb: (to: 'A' | 'B') => void
  enabled: boolean
  onBypass: (on: boolean) => void
  presetPicker: ReactNode
  rootRef: RefObject<HTMLDivElement>
  menu: ReactNode
  onClose?: () => void
  onHeaderPointerDown?: (e: ReactPointerEvent) => void
}

type Shape = 'lowcut' | 'lowshelf' | 'bell' | 'notch' | 'highshelf' | 'highcut'
/** In the order the plug-in's Type parameter counts them. */
const SHAPES: Shape[] = ['lowcut', 'lowshelf', 'bell', 'notch', 'highshelf', 'highcut']
const SHAPE_NAME: Record<Shape, string> = {
  lowcut: 'Low cut', lowshelf: 'Low shelf', bell: 'Bell', notch: 'Notch', highshelf: 'High shelf', highcut: 'High cut',
}
const SHAPE_ICON: Record<Shape, string> = {
  lowcut: 'M2 13 C5 13 6 4 9 4 L16 4',
  lowshelf: 'M2 6 L6 6 C8 6 9 11 11 11 L16 11',
  bell: 'M2 11 C6 11 7 3 9 3 C11 3 12 11 16 11',
  notch: 'M2 5 L6 5 C8 5 8.5 13 9 13 C9.5 13 10 5 12 5 L16 5',
  highshelf: 'M2 11 L7 11 C9 11 10 6 12 6 L16 6',
  highcut: 'M2 4 L9 4 C12 4 13 13 16 13',
}
/** How each shape is computed: the cuts are two filters in a row (24 dB an octave). */
const SHAPE_KIND: Record<Shape, [BiquadKind, number]> = {
  lowcut: ['highpass', 2], lowshelf: ['lowshelf', 1], bell: ['peak', 1], notch: ['notch', 1], highshelf: ['highshelf', 1], highcut: ['lowpass', 2],
}
const NO_GAIN = new Set<Shape>(['lowcut', 'highcut', 'notch'])
const BAND_COLOURS = ['#ff5a6e', '#ffa94d', '#ffd84d', '#5be38f', '#4dabf7', '#b197fc', '#f783ac']

interface Band {
  index: number
  on: boolean
  f: number
  g: number
  q: number
  shape: Shape
  p: { on?: PluginParam; f?: PluginParam; g?: PluginParam; q?: PluginParam; type?: PluginParam }
}

function readBands(view: ParamView): Band[] {
  const count = view.params.filter((x) => /^Band \d+ Enabled$/.test(x.name)).length
  return Array.from({ length: count }, (_, i) => {
    const n = i + 1
    const type = view.par(`Band ${n} Type`)
    return {
      index: i,
      on: view.v(`Band ${n} Enabled`, 0) >= 0.5,
      f: view.v(`Band ${n} Frequency`, 1000),
      g: view.v(`Band ${n} Gain`, 0),
      q: view.v(`Band ${n} Q`, 1),
      shape: type ? SHAPES[Math.round(view.v(`Band ${n} Type`, 2))] ?? 'bell' : i === 0 ? 'lowshelf' : i === count - 1 ? 'highshelf' : 'bell',
      p: {
        on: view.par(`Band ${n} Enabled`), f: view.par(`Band ${n} Frequency`), g: view.par(`Band ${n} Gain`),
        q: view.par(`Band ${n} Q`), type,
      },
    }
  })
}

function bandDb(b: Band, hz: number, sr: number): number {
  const [kind, stages] = SHAPE_KIND[b.shape]
  return biquadDb(kind, b.f, b.q, NO_GAIN.has(b.shape) ? 0 : b.g, hz, sr) * stages
}

const FMIN = 20, FMAX = 20000
const fmtF = (f: number) => (f >= 1000 ? `${(f / 1000).toFixed(f >= 10000 ? 1 : 2)} kHz` : `${Math.round(f)} Hz`)
const fmtG = (d: number) => `${d > 0 ? '+' : ''}${d.toFixed(1)} dB`
const GRID_F = [20, 30, 40, 50, 60, 70, 80, 90, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000, 20000]
const LABEL_F: [number, string][] = [[50, '50'], [100, '100'], [200, '200'], [500, '500'], [1000, '1k'], [2000, '2k'], [5000, '5k'], [10000, '10k']]

export function EqWindow(props: EqWindowProps) {
  const { view, setParam, openMenu, live, bus, sampleRate } = props
  const bands = useMemo(() => readBands(view), [view])
  const outQ = view.par('Output Gain')
  const [sel, setSel] = useState(() => bands.findIndex((b) => b.on))
  const [hover, setHover] = useState(-1)
  const [range, setRange] = useState(18)
  const [analyzer, setAnalyzer] = useState(true)
  const [tip, setTip] = useState<{ x: number; y: number; text: string } | null>(null)
  const dispRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const meterRef = useRef<HTMLCanvasElement>(null)
  const size = useRef({ W: 0, H: 0 })

  // ---- geometry
  const xOf = useCallback((f: number) => (Math.log(f / FMIN) / Math.log(FMAX / FMIN)) * size.current.W, [])
  const fOf = useCallback((x: number) => FMIN * Math.pow(FMAX / FMIN, Math.max(0, Math.min(1, x / size.current.W))), [])
  const yOf = useCallback((db: number) => size.current.H / 2 - (db / range) * (size.current.H / 2 - 18), [range])
  const dbOf = useCallback((y: number) => ((size.current.H / 2 - y) / (size.current.H / 2 - 18)) * range, [range])
  const nodeY = useCallback((b: Band) => (NO_GAIN.has(b.shape) ? yOf(0) : yOf(b.g)), [yOf])

  // ---- the analyzer, smoothed here: fast up, slow down, like a meter
  const smooth = useRef<{ pre: Float32Array; post: Float32Array } | null>(null)
  const takeSpectrum = useCallback(() => {
    const sp = live.spectrum
    if (!sp) return
    const [pre, post] = sp
    if (!smooth.current || smooth.current.pre.length !== pre.length) {
      smooth.current = { pre: Float32Array.from(pre), post: Float32Array.from(post) }
      return
    }
    const s = smooth.current
    for (let i = 0; i < pre.length; i++) {
      s.pre[i] += (pre[i] - s.pre[i]) * (pre[i] > s.pre[i] ? 0.6 : 0.15)
      s.post[i] += (post[i] - s.post[i]) * (post[i] > s.post[i] ? 0.6 : 0.15)
    }
  }, [live])

  // ---- drawing
  const latest = useRef({ bands, sel, hover, range, analyzer, enabled: props.enabled })
  latest.current = { bands, sel, hover, range, analyzer, enabled: props.enabled }
  const draw = useCallback(() => {
    const c = canvasRef.current
    if (!c) return
    const { W, H } = size.current
    const g = c.getContext('2d')
    if (!g || W === 0) return
    const dpr = window.devicePixelRatio || 1
    g.setTransform(dpr, 0, 0, dpr, 0, 0)
    g.clearRect(0, 0, W, H)
    const L = latest.current
    // grid
    g.font = '500 10px Inter, system-ui, sans-serif'
    g.textBaseline = 'middle'
    for (const f of GRID_F) {
      const x = Math.round(xOf(f)) + 0.5
      g.strokeStyle = f === 100 || f === 1000 || f === 10000 ? 'rgba(255,255,255,.075)' : 'rgba(255,255,255,.03)'
      g.beginPath(); g.moveTo(x, 0); g.lineTo(x, H); g.stroke()
    }
    g.fillStyle = '#5d606b'; g.textAlign = 'center'
    for (const [f, l] of LABEL_F) g.fillText(l, xOf(f), H - 10)
    const step = L.range > 12 ? 6 : 3
    g.textAlign = 'right'
    for (let d = -L.range; d <= L.range; d += step) {
      const y = Math.round(yOf(d)) + 0.5
      g.strokeStyle = d === 0 ? 'rgba(255,255,255,.11)' : 'rgba(255,255,255,.035)'
      g.beginPath(); g.moveTo(0, y); g.lineTo(W, y); g.stroke()
      if (d !== -L.range && d !== L.range) g.fillText(`${d > 0 ? '+' : ''}${d}`, W - 8, y - 7)
    }
    // analyzer: before as a faint fill, after as a lit one
    const sp = smooth.current
    if (L.analyzer && sp) {
      const n = sp.pre.length
      const sy = (d: number) => H - ((Math.max(-96, Math.min(0, d)) + 96) / 96) * H * 0.92
      const fill = (arr: Float32Array) => {
        g.beginPath(); g.moveTo(0, H)
        for (let i = 0; i < n; i++) g.lineTo((i / (n - 1)) * W, sy(arr[i]))
        g.lineTo(W, H); g.closePath()
      }
      fill(sp.pre); g.fillStyle = 'rgba(255,255,255,.035)'; g.fill()
      fill(sp.post)
      const gr = g.createLinearGradient(0, 0, 0, H)
      gr.addColorStop(0, 'rgba(120,170,255,.28)'); gr.addColorStop(1, 'rgba(120,170,255,.02)')
      g.fillStyle = gr; g.fill()
      g.beginPath()
      for (let i = 0; i < n; i++) g.lineTo((i / (n - 1)) * W, sy(sp.post[i]))
      g.strokeStyle = 'rgba(150,190,255,.45)'; g.lineWidth = 1; g.stroke()
    }
    // the hovered or selected band's own curve, in its colour
    const focus = L.hover >= 0 ? L.hover : L.sel
    const fb = L.bands[focus]
    if (fb && fb.on) {
      const col = BAND_COLORS(focus)
      g.beginPath(); g.moveTo(0, yOf(0))
      for (let x = 0; x <= W; x += 2) g.lineTo(x, yOf(clampDb(bandDb(fb, fOf(x), sampleRate), L.range)))
      g.lineTo(W, yOf(0)); g.closePath()
      g.fillStyle = `${col}2a`; g.fill(); g.strokeStyle = `${col}99`; g.lineWidth = 1; g.stroke()
    }
    // the response of every band together
    g.beginPath()
    for (let x = 0; x <= W; x += 1.5) {
      let d = 0
      if (L.enabled) for (const b of L.bands) if (b.on) d += bandDb(b, fOf(x), sampleRate)
      g.lineTo(x, yOf(clampDb(d, L.range * 1.3)))
    }
    g.strokeStyle = L.enabled ? '#f2f3f6' : '#5d606b'; g.lineWidth = 2
    g.shadowColor = 'rgba(255,255,255,.35)'; g.shadowBlur = L.enabled ? 8 : 0; g.stroke(); g.shadowBlur = 0
    // points
    g.textAlign = 'center'; g.textBaseline = 'middle'
    L.bands.forEach((b, i) => {
      // A band that is off is not there, as in any EQ: it shows only while
      // it is the one being edited.
      if (!b.on && i !== L.sel) return
      const x = xOf(b.f), y = nodeY(b), col = BAND_COLORS(i)
      const r = i === L.sel ? 9 : i === L.hover ? 8.5 : 7.5
      if (i === L.sel) { g.beginPath(); g.arc(x, y, r + 5, 0, Math.PI * 2); g.strokeStyle = `${col}66`; g.lineWidth = 1.5; g.stroke() }
      g.beginPath(); g.arc(x, y, r, 0, Math.PI * 2)
      g.fillStyle = b.on ? col : '#2a2c33'
      g.shadowColor = col; g.shadowBlur = b.on && (i === L.sel || i === L.hover) ? 14 : 0; g.fill(); g.shadowBlur = 0
      g.strokeStyle = 'rgba(0,0,0,.55)'; g.lineWidth = 1; g.stroke()
      g.fillStyle = b.on ? '#101114' : '#9a9ca7'; g.font = '700 10px Inter, system-ui, sans-serif'
      g.fillText(String(i + 1), x, y + 0.5)
    })
  }, [xOf, yOf, fOf, nodeY, sampleRate])

  // size the canvas to its box, and redraw
  useLayoutEffect(() => {
    const el = dispRef.current, c = canvasRef.current
    if (!el || !c) return
    const fit = () => {
      const dpr = window.devicePixelRatio || 1
      size.current = { W: el.clientWidth, H: el.clientHeight }
      c.width = Math.round(size.current.W * dpr); c.height = Math.round(size.current.H * dpr)
      draw()
    }
    fit()
    const ro = new ResizeObserver(fit)
    ro.observe(el)
    return () => ro.disconnect()
  }, [draw])
  useEffect(() => { draw() }, [draw, bands, sel, hover, range, analyzer, props.enabled])

  // live: new measurements redraw the display and the meter, nothing else
  const meterState = useRef({ l: -60, r: -60 })
  useEffect(() => bus.subscribe(() => {
    takeSpectrum()
    draw()
    drawMeter(meterRef.current, live.levels[2], live.levels[3], meterState.current)
  }), [bus, takeSpectrum, draw, live])

  // ---- interaction on the display
  const nodeAt = useCallback((x: number, y: number) => {
    let best = -1, bd = 14
    bands.forEach((b, i) => {
      if (!b.on && i !== sel) return
      const d = Math.hypot(xOf(b.f) - x, nodeY(b) - y)
      if (d < bd) { bd = d; best = i }
    })
    return best
  }, [bands, sel, xOf, nodeY])
  const local = (e: { clientX: number; clientY: number }) => {
    const r = dispRef.current!.getBoundingClientRect()
    return [e.clientX - r.left, e.clientY - r.top] as const
  }
  const drag = useRef<{ i: number } | null>(null)
  const bandsRef = useRef(bands)
  bandsRef.current = bands

  const moveBand = (i: number, x: number, y: number, fine: boolean) => {
    const b = bandsRef.current[i]
    if (!b) return
    if (b.p.f) setParam(b.p.f, fine ? b.f * Math.pow(fOf(x) / b.f, 0.2) : fOf(x))
    if (b.p.g && !NO_GAIN.has(b.shape)) setParam(b.p.g, Math.round(Math.max(-range, Math.min(range, dbOf(y))) * 10) / 10)
    if (!b.on && b.p.on) setParam(b.p.on, b.p.on.max)
  }

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest('.eqw-pop')) return
    const [x, y] = local(e)
    const i = nodeAt(x, y)
    if (i < 0) { setSel(-1); return }
    setSel(i)
    drag.current = { i }
    e.currentTarget.setPointerCapture(e.pointerId)
  }
  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    if ((e.target as HTMLElement).closest('.eqw-pop') && !drag.current) { setTip(null); return }
    const [x, y] = local(e)
    if (drag.current) moveBand(drag.current.i, x, y, e.shiftKey)
    const i = drag.current ? drag.current.i : nodeAt(x, y)
    setHover(i)
    const b = i >= 0 ? bandsRef.current[i] : null
    const text = b
      ? `${SHAPE_NAME[b.shape]} · ${fmtF(b.f)}${NO_GAIN.has(b.shape) ? '' : ` · ${fmtG(b.g)}`} · Q ${b.q.toFixed(2)}`
      : `${fmtF(fOf(x))} · ${fmtG(dbOf(y))}`
    setTip({ x, y, text })
  }
  const onPointerUp = () => { drag.current = null }
  const onPointerLeave = () => { setHover(-1); setTip(null) }
  const onWheel = (e: React.WheelEvent<HTMLDivElement>) => {
    const [x, y] = local(e)
    const at = nodeAt(x, y)
    const i = at >= 0 ? at : sel
    const b = bands[i]
    if (!b?.p.q) return
    setSel(i)
    setParam(b.p.q, Math.max(b.p.q.min, Math.min(b.p.q.max, b.q * Math.exp(-e.deltaY * (e.shiftKey ? 0.0004 : 0.0015)))))
  }
  // A double-click on an empty spot turns on the first band that is off,
  // there, as a bell; on a point it turns that band off.
  const onDoubleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if ((e.target as HTMLElement).closest('.eqw-pop')) return
    const [x, y] = local(e)
    const at = nodeAt(x, y)
    if (at >= 0) { const b = bands[at]; if (b.p.on) setParam(b.p.on, b.on ? b.p.on.min : b.p.on.max); return }
    const free = bands.find((b) => !b.on)
    if (!free) return
    if (free.p.type) setParam(free.p.type, SHAPES.indexOf('bell'))
    if (free.p.q) setParam(free.p.q, 1)
    moveBand(free.index, x, y, false)
    setSel(free.index)
  }
  // Right-click a point: the parameter menu for its frequency (automation,
  // MIDI learn, type a value).
  const onContextMenu = (e: React.MouseEvent<HTMLDivElement>) => {
    const [x, y] = local(e)
    const at = nodeAt(x, y)
    const q = at >= 0 ? bands[at].p.f : undefined
    if (q) openMenu(e, q)
    else e.preventDefault()
  }
  // Keep the display from scrolling the page while the wheel sets Q.
  useEffect(() => {
    const el = dispRef.current
    if (!el) return
    const stop = (ev: WheelEvent) => ev.preventDefault()
    el.addEventListener('wheel', stop, { passive: false })
    return () => el.removeEventListener('wheel', stop)
  }, [])

  const selBand = bands[sel]

  return (
    <div ref={props.rootRef} className="eqw" onContextMenu={(e) => e.preventDefault()}>
      <div className="eqw-hd" onPointerDown={props.onHeaderPointerDown}>
        <div className="eqw-logo"><svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="#fff" strokeWidth="1.6" strokeLinecap="round"><path d="M1 8c2 0 2-5 4-5s2 7 4 7 1.5-3 2-3" /></svg></div>
        <span className="eqw-nm">{props.name}</span>
        <div className="eqw-pre" onPointerDown={(e) => e.stopPropagation()}>{props.presetPicker}</div>
        <div className="eqw-sep" />
        <div className="eqw-ab" onPointerDown={(e) => e.stopPropagation()}>
          <button className={props.ab === 'A' ? 'on' : ''} onClick={() => props.onAb('A')} title="Settings A">A</button>
          <button className={props.ab === 'B' ? 'on' : ''} onClick={() => props.onAb('B')} title="Settings B, to compare with A">B</button>
        </div>
        <div className="eqw-grow" />
        <div className="eqw-tools" onPointerDown={(e) => e.stopPropagation()}>
          <button onClick={() => setRange((r) => (r === 18 ? 12 : r === 12 ? 30 : 18))} title="Gain range of the display">±{range} dB</button>
          <button className={analyzer ? 'on' : ''} onClick={() => setAnalyzer((a) => !a)} title="Analyzer: the sound before and after the EQ">Analyzer</button>
          <div className="eqw-sep" />
          <button className={`eqw-pw${props.enabled ? '' : ' off'}`} onClick={() => props.onBypass(!props.enabled)} title={props.enabled ? 'Bypass' : 'Turn on'}>
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" strokeWidth="1.6" strokeLinecap="round"><path d="M6 1.2v4" /><path d="M3.2 3a4 4 0 1 0 5.6 0" /></svg>
          </button>
          {props.onClose && <button className="eqw-x" onClick={props.onClose} aria-label="Close" title="Close (Esc)">✕</button>}
        </div>
      </div>

      <div className="eqw-main">
        <div
          ref={dispRef} className="eqw-disp"
          onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerLeave={onPointerLeave}
          onWheel={onWheel} onDoubleClick={onDoubleClick} onContextMenu={onContextMenu}
        >
          <canvas ref={canvasRef} />
          {props.error && <div className="eqw-note">Could not read the EQ's settings: {props.error}</div>}
          {props.ready && !bands.some((b) => b.on) && sel < 0 && <div className="eqw-empty">Double-click to add a band</div>}
          {!props.ready && !props.error && <div className="eqw-note">Loading…</div>}
          {tip && <div className="eqw-tip" style={{ left: Math.min(size.current.W - 220, tip.x + 14), top: Math.max(6, tip.y - 30) }}>{tip.text}</div>}
          {selBand && selBand.p.f && (
            <BandPop
              band={selBand} colour={BAND_COLORS(sel)} x={xOf(selBand.f)} y={nodeY(selBand)} W={size.current.W} H={size.current.H}
              setParam={setParam} openMenu={openMenu} automated={props.automated}
            />
          )}
        </div>
        <div className="eqw-side">
          <span className="eqw-lab">Out</span>
          <canvas ref={meterRef} className="eqw-meter" />
          {outQ && (
            <Knob
              q={outQ} value={view.v('Output Gain', 0)} label="" colour="#e6e7eb" bipolar text={fmtG(view.v('Output Gain', 0))}
              setParam={setParam} openMenu={openMenu} auto={props.automated.has(outQ.id)}
            />
          )}
        </div>
      </div>

      <div className="eqw-ft">
        {bands.map((b, i) => (b.on || i === sel) && (
          <span key={i} className={`eqw-chip${i === sel ? ' sel' : ''}${b.on ? '' : ' off'}`} onClick={() => setSel(i)} title={`${SHAPE_NAME[b.shape]}, band ${i + 1}`}>
            <i style={{ background: BAND_COLORS(i) }} />{fmtF(b.f)}{NO_GAIN.has(b.shape) ? '' : ` · ${fmtG(b.g)}`}
          </span>
        ))}
        <span className="eqw-hint">{bands.some((b) => !b.on) ? 'Double-click to add a band · drag a point · scroll for Q · right-click for automation' : 'Drag a point · scroll for Q · right-click for automation'}</span>
      </div>
      {props.menu}
    </div>
  )
}

const BAND_COLORS = (i: number) => BAND_COLOURS[i % BAND_COLOURS.length]
const clampDb = (d: number, r: number) => Math.max(-r * 1.2, Math.min(r * 1.2, d))

/** What belongs to one band, beside its point. */
function BandPop({ band, colour, x, y, W, H, setParam, openMenu, automated }: {
  band: Band; colour: string; x: number; y: number; W: number; H: number
  setParam: (q: PluginParam, v: number) => void
  openMenu: (e: React.MouseEvent, q: PluginParam) => void
  automated: Set<number>
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: 0, top: 0 })
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const pw = el.offsetWidth, ph = el.offsetHeight
    let top = y + 22
    if (top + ph > H - 26) top = y - ph - 22
    setPos({ left: Math.max(8, Math.min(W - pw - 8, x - pw / 2)), top: Math.max(8, top) })
  }, [x, y, W, H, band.shape])
  const noGain = NO_GAIN.has(band.shape)
  return (
    <div ref={ref} className="eqw-pop" style={{ left: pos.left, top: pos.top, ['--bc' as string]: colour }} onPointerDown={(e) => e.stopPropagation()}>
      {band.p.type && (
        <div className="eqw-types">
          {SHAPES.map((s, i) => (
            <button key={s} className={band.shape === s ? 'on' : ''} title={SHAPE_NAME[s]} onClick={() => band.p.type && setParam(band.p.type, i)}>
              <svg width="18" height="16" viewBox="0 0 18 16" fill="none" strokeWidth="1.6" strokeLinecap="round"><path d={SHAPE_ICON[s]} /></svg>
            </button>
          ))}
        </div>
      )}
      {band.p.f && <Knob q={band.p.f} value={band.f} label="Freq" colour={colour} log text={fmtF(band.f)} setParam={setParam} openMenu={openMenu} auto={automated.has(band.p.f.id)} />}
      {band.p.g && <Knob q={band.p.g} value={band.g} label="Gain" colour={colour} bipolar text={fmtG(band.g)} disabled={noGain} setParam={setParam} openMenu={openMenu} auto={automated.has(band.p.g.id)} />}
      {band.p.q && <Knob q={band.p.q} value={band.q} label="Q" colour={colour} log text={band.q.toFixed(2)} setParam={setParam} openMenu={openMenu} auto={automated.has(band.p.q.id)} />}
      {band.p.on && (
        <button className={`eqw-pw${band.on ? '' : ' off'}`} title={band.on ? 'Turn this band off' : 'Turn this band on'} onClick={() => band.p.on && setParam(band.p.on, band.on ? band.p.on.min : band.p.on.max)}>
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" strokeWidth="1.6" strokeLinecap="round"><path d="M6 1.2v4" /><path d="M3.2 3a4 4 0 1 0 5.6 0" /></svg>
        </button>
      )}
    </div>
  )
}

/**
 * A small knob: drag up and down (Shift for fine), double-click for its
 * default, right-click for the parameter menu.
 */
function Knob({ q, value, label, colour, text, log, bipolar, disabled, setParam, openMenu, auto }: {
  q: PluginParam; value: number; label: string; colour: string; text: string
  log?: boolean; bipolar?: boolean; disabled?: boolean; auto?: boolean
  setParam: (q: PluginParam, v: number) => void
  openMenu: (e: React.MouseEvent, q: PluginParam) => void
}) {
  const ref = useRef<HTMLCanvasElement>(null)
  const toT = useCallback((v: number) => (log
    ? Math.log(Math.max(q.min, v) / Math.max(1e-6, q.min)) / Math.log(q.max / Math.max(1e-6, q.min))
    : (v - q.min) / Math.max(1e-9, q.max - q.min)), [q, log])
  const fromT = useCallback((t: number) => {
    const c = Math.max(0, Math.min(1, t))
    return log ? q.min * Math.pow(q.max / q.min, c) : q.min + c * (q.max - q.min)
  }, [q, log])
  useEffect(() => { if (ref.current) drawKnob(ref.current, toT(value), colour, !!bipolar) }, [value, colour, bipolar, toT])
  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || disabled) return
    e.stopPropagation()
    const y0 = e.clientY, t0 = toT(value)
    const move = (ev: PointerEvent) => setParam(q, fromT(t0 + ((y0 - ev.clientY) / 160) * (ev.shiftKey ? 0.2 : 1)))
    const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up) }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }
  return (
    <div
      className={`eqw-k${disabled ? ' dis' : ''}${auto ? ' auto' : ''}`}
      onPointerDown={onPointerDown}
      onDoubleClick={() => !disabled && setParam(q, q.defaultValue)}
      onContextMenu={(e) => openMenu(e, q)}
      title={`${q.name}: ${text}`}
    >
      <canvas ref={ref} />
      <b>{text}</b>
      {label && <small>{label}</small>}
    </div>
  )
}

function drawKnob(c: HTMLCanvasElement, t: number, col: string, bipolar: boolean) {
  const s = 32, dpr = window.devicePixelRatio || 1
  c.width = s * dpr; c.height = s * dpr
  const g = c.getContext('2d')
  if (!g) return
  g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, s, s)
  const a0 = Math.PI * 0.75, sw = Math.PI * 1.5, av = a0 + sw * Math.max(0, Math.min(1, t))
  g.lineCap = 'round'; g.lineWidth = 2.5
  g.strokeStyle = '#2a2c34'; g.beginPath(); g.arc(16, 16, 13, a0, a0 + sw); g.stroke()
  g.strokeStyle = col; g.beginPath()
  if (bipolar) { const m = a0 + sw / 2; g.arc(16, 16, 13, Math.min(m, av), Math.max(m, av)) } else g.arc(16, 16, 13, a0, av)
  g.stroke()
  const gr = g.createLinearGradient(0, 4, 0, 28)
  gr.addColorStop(0, '#34363f'); gr.addColorStop(1, '#1b1c21')
  g.fillStyle = gr; g.beginPath(); g.arc(16, 16, 9.5, 0, Math.PI * 2); g.fill()
  g.strokeStyle = 'rgba(255,255,255,.08)'; g.lineWidth = 1; g.beginPath(); g.arc(16, 16, 9.5, Math.PI * 1.15, Math.PI * 1.85); g.stroke()
  g.strokeStyle = '#fff'; g.lineWidth = 1.8
  g.beginPath(); g.moveTo(16 + Math.cos(av) * 3, 16 + Math.sin(av) * 3); g.lineTo(16 + Math.cos(av) * 8, 16 + Math.sin(av) * 8); g.stroke()
}

/** The output meter: two bars with a fast rise and a slow fall. */
function drawMeter(c: HTMLCanvasElement | null, l: number, r: number, st: { l: number; r: number }) {
  if (!c) return
  const dpr = window.devicePixelRatio || 1, w = c.clientWidth, h = c.clientHeight
  if (w === 0 || h === 0) return
  if (c.width !== Math.round(w * dpr)) { c.width = Math.round(w * dpr); c.height = Math.round(h * dpr) }
  const g = c.getContext('2d')
  if (!g) return
  g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, w, h)
  const db = (x: number) => (x > 1e-6 ? 20 * Math.log10(x) : -60)
  const dl = db(l), dr = db(r)
  st.l = dl > st.l ? dl : st.l - 1.2
  st.r = dr > st.r ? dr : st.r - 1.2
  const y = (d: number) => h - ((Math.max(-60, Math.min(6, d)) + 60) / 66) * h
  const gr = g.createLinearGradient(0, h, 0, 0)
  gr.addColorStop(0, '#2fb67a'); gr.addColorStop(0.72, '#5be38f'); gr.addColorStop(0.86, '#ffd84d'); gr.addColorStop(1, '#ff5a6e')
  const cw = w / 2 - 1
  for (const [i, v] of [[0, st.l], [1, st.r]] as const) {
    const x = i * (w / 2 + 1)
    g.fillStyle = '#0c0d10'; g.fillRect(x, 0, cw, h)
    g.fillStyle = gr; g.fillRect(x, y(v), cw, h - y(v))
  }
  g.fillStyle = 'rgba(0,0,0,.35)'
  for (let k = 0; k < h; k += 3) g.fillRect(0, k, w, 1)
}
