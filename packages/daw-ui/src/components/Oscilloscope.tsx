import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'

interface OscilloscopeProps {
  onClose: () => void
}

type Mode = 'overlay' | 'split' | 'xy'
type Trigger = 'off' | 'rising'

const DEFAULT_WINDOW_MS = 40
const MIN_WINDOW_MS = 5
const MAX_WINDOW_MS = 500
const TAP_CAPACITY_FRAMES = 8192

const POLL_INTERVAL_MS = 33

function findRisingZeroCross(left: Float32Array): number {
  for (let i = 1; i < left.length; i++) {
    if (left[i - 1] <= 0 && left[i] > 0) return i
  }
  return 0
}

export function Oscilloscope({ onClose }: OscilloscopeProps) {
  const [windowMs, setWindowMs] = useState(DEFAULT_WINDOW_MS)
  const [mode, setMode] = useState<Mode>('overlay')
  const [trigger, setTrigger] = useState<Trigger>('rising')
  const [freeze, setFreeze] = useState(false)
  const [sampleRate, setSampleRate] = useState(48000)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const leftRef = useRef<Float32Array>(new Float32Array(0))
  const rightRef = useRef<Float32Array>(new Float32Array(0))

  useEffect(() => {
    let cancelled = false
    invoke<{ device: string | null; sample_rate: number; buffer_size: number }>('get_audio_config')
      .then(cfg => {
        if (!cancelled && cfg.sample_rate > 0) setSampleRate(cfg.sample_rate)
      })
      .catch(() => {})
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    if (freeze) return
    const frames = Math.min(
      TAP_CAPACITY_FRAMES,
      Math.max(64, Math.round((windowMs / 1000) * sampleRate))
    )
    const id = setInterval(async () => {
      try {
        const samples = await invoke<number[]>('get_master_samples', { nFrames: frames })
        const n = Math.floor(samples.length / 2)
        if (n === 0) return
        const l = new Float32Array(n)
        const r = new Float32Array(n)
        for (let i = 0; i < n; i++) {
          l[i] = samples[i * 2]
          r[i] = samples[i * 2 + 1]
        }
        leftRef.current = l
        rightRef.current = r
        const c = canvasRef.current
        if (c) draw(c, l, r, mode, trigger)
      } catch { /* ignore */ }
    }, POLL_INTERVAL_MS)
    return () => clearInterval(id)
  }, [windowMs, sampleRate, freeze, mode, trigger])

  useEffect(() => {
    const c = canvasRef.current
    if (!c) return
    draw(c, leftRef.current, rightRef.current, mode, trigger)
  }, [mode, trigger])

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 9800,
        background: 'rgba(0,0,0,0.45)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div
        style={{
          width: 680, maxWidth: '94vw',
          background: hw.bg, color: hw.textPrimary,
          border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg,
          overflow: 'hidden',
        }}
      >
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center',
          gap: 12, background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Oscilloscope</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>Master bus · {sampleRate} Hz</div>
          <div style={{ flex: 1 }} />
          <button onClick={() => setFreeze(v => !v)} style={{
            padding: '3px 10px', fontSize: 10, background: 'transparent',
            border: `1px solid ${freeze ? hw.accent : hw.border}`, borderRadius: hw.radius.sm,
            color: freeze ? hw.accent : hw.textSecondary, cursor: 'pointer',
            transition: 'color 0.15s, border-color 0.15s',
          }}>
            {freeze ? 'Frozen' : 'Freeze'}
          </button>
          <button onClick={onClose} style={{
            padding: '3px 10px', fontSize: 10, background: 'transparent',
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
            color: hw.textSecondary, cursor: 'pointer',
          }}>
            Close
          </button>
        </div>

        <canvas
          ref={canvasRef}
          width={680}
          height={320}
          style={{ width: '100%', height: 320, background: '#0a0a0e', display: 'block' }}
        />

        <div style={{
          padding: '8px 12px', borderTop: `1px solid ${hw.border}`,
          background: hw.bgElevated, display: 'flex', alignItems: 'center',
          gap: 16, flexWrap: 'wrap',
        }}>
          <label style={{ fontSize: 10, color: hw.textSecondary, display: 'flex', gap: 8, alignItems: 'center' }}>
            <span>Window</span>
            <input
              type="range"
              min={MIN_WINDOW_MS}
              max={MAX_WINDOW_MS}
              step={1}
              value={windowMs}
              onChange={(e) => setWindowMs(parseInt(e.target.value, 10))}
              style={{ width: 140 }}
              disabled={mode === 'xy'}
            />
            <span style={{ color: hw.textPrimary, fontVariantNumeric: 'tabular-nums', minWidth: 48 }}>
              {windowMs} ms
            </span>
          </label>

          <label style={{ fontSize: 10, color: hw.textSecondary, display: 'flex', gap: 6, alignItems: 'center' }}>
            <span>Mode</span>
            <select
              value={mode}
              onChange={(e) => setMode(e.target.value as Mode)}
              style={{
                fontSize: 10, padding: '2px 6px', background: hw.bg,
                border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
                color: hw.textPrimary,
              }}
            >
              <option value="overlay">Overlay L+R</option>
              <option value="split">Split (L/R)</option>
              <option value="xy">Lissajous (X-Y)</option>
            </select>
          </label>

          <label style={{ fontSize: 10, color: hw.textSecondary, display: 'flex', gap: 6, alignItems: 'center' }}>
            <span>Trigger</span>
            <select
              value={trigger}
              onChange={(e) => setTrigger(e.target.value as Trigger)}
              disabled={mode === 'xy'}
              style={{
                fontSize: 10, padding: '2px 6px', background: hw.bg,
                border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
                color: hw.textPrimary,
              }}
            >
              <option value="off">Free-run</option>
              <option value="rising">Rising zero-cross</option>
            </select>
          </label>

          <div style={{ flex: 1 }} />
          <div style={{ fontSize: 9, color: hw.textFaint }}>
            <span style={{ color: hw.accent }}>━</span> L &nbsp;
            <span style={{ color: hw.secondary }}>━</span> R
          </div>
        </div>
      </div>
    </div>
  )
}

function draw(
  c: HTMLCanvasElement,
  left: Float32Array,
  right: Float32Array,
  mode: Mode,
  trigger: Trigger,
) {
  const ctx = c.getContext('2d')
  if (!ctx) return
  const dpr = window.devicePixelRatio || 1
  const w = c.clientWidth
  const h = c.clientHeight
  if (c.width !== Math.floor(w * dpr) || c.height !== Math.floor(h * dpr)) {
    c.width = Math.floor(w * dpr)
    c.height = Math.floor(h * dpr)
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx.fillStyle = '#0a0a0e'
  ctx.fillRect(0, 0, w, h)

  if (left.length === 0) {
    ctx.fillStyle = hw.textFaint
    ctx.font = '10px sans-serif'
    ctx.fillText('Waiting for signal...', 12, 18)
    return
  }

  if (mode === 'xy') {
    drawXY(ctx, w, h, left, right)
    return
  }

  let start = 0
  if (trigger === 'rising') start = findRisingZeroCross(left)
  const available = left.length - start
  const visible = Math.max(1, available)

  if (mode === 'split') {
    drawGrid(ctx, w, h, true)
    drawWave(ctx, w, h / 2, left, start, visible, hw.accent, 0)
    drawWave(ctx, w, h / 2, right, start, visible, hw.secondary, h / 2)
  } else {
    drawGrid(ctx, w, h, false)
    drawWave(ctx, w, h, left, start, visible, hw.accent, 0)
    drawWave(ctx, w, h, right, start, visible, hw.secondary, 0)
  }
}

function drawGrid(ctx: CanvasRenderingContext2D, w: number, h: number, split: boolean) {
  ctx.strokeStyle = hw.border
  ctx.lineWidth = 1
  if (split) {
    const mids = [h / 4, (3 * h) / 4]
    for (const y of mids) {
      ctx.beginPath()
      ctx.moveTo(0, y)
      ctx.lineTo(w, y)
      ctx.stroke()
    }
    ctx.strokeStyle = hw.textFaint
    ctx.beginPath()
    ctx.moveTo(0, h / 2)
    ctx.lineTo(w, h / 2)
    ctx.stroke()
  } else {
    ctx.beginPath()
    ctx.moveTo(0, h / 2)
    ctx.lineTo(w, h / 2)
    ctx.stroke()
  }
}

function drawWave(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  data: Float32Array,
  start: number,
  visible: number,
  color: string,
  yOffset: number,
) {
  ctx.strokeStyle = color
  ctx.lineWidth = 1.25
  ctx.beginPath()
  const mid = yOffset + h / 2
  for (let i = 0; i < visible; i++) {
    const s = data[start + i]
    const x = (i / (visible - 1)) * w
    const y = mid - Math.max(-1, Math.min(1, s)) * (h / 2 - 2)
    if (i === 0) ctx.moveTo(x, y)
    else ctx.lineTo(x, y)
  }
  ctx.stroke()
}

function drawXY(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  left: Float32Array,
  right: Float32Array,
) {
  ctx.strokeStyle = hw.border
  ctx.lineWidth = 1
  ctx.beginPath()
  ctx.moveTo(w / 2, 0)
  ctx.lineTo(w / 2, h)
  ctx.moveTo(0, h / 2)
  ctx.lineTo(w, h / 2)
  ctx.stroke()

  const cx = w / 2
  const cy = h / 2
  const r = Math.min(w, h) / 2 - 4

  ctx.strokeStyle = hw.accent
  ctx.lineWidth = 1
  ctx.beginPath()
  const n = Math.min(left.length, right.length)
  for (let i = 0; i < n; i++) {
    const x = cx + Math.max(-1, Math.min(1, left[i])) * r
    const y = cy - Math.max(-1, Math.min(1, right[i])) * r
    if (i === 0) ctx.moveTo(x, y)
    else ctx.lineTo(x, y)
  }
  ctx.stroke()
}
