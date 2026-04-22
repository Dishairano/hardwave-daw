import { useEffect, useMemo, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'

interface SpectrumAnalyzerProps {
  onClose: () => void
}

type FftSize = 1024 | 2048 | 4096 | 8192
type WindowType = 'hanning' | 'blackman-harris' | 'flat-top'
type DisplayStyle = 'bars' | 'line'
type FreqScale = 'log' | 'linear'
type ChannelMode = 'lr' | 'mid-side'

const POLL_INTERVAL_MS = 33
const MIN_DB = -100
const MAX_DB = 0

function hanning(n: number, N: number): number {
  return 0.5 - 0.5 * Math.cos((2 * Math.PI * n) / (N - 1))
}

function blackmanHarris(n: number, N: number): number {
  const a0 = 0.35875
  const a1 = 0.48829
  const a2 = 0.14128
  const a3 = 0.01168
  const t = (2 * Math.PI * n) / (N - 1)
  return a0 - a1 * Math.cos(t) + a2 * Math.cos(2 * t) - a3 * Math.cos(3 * t)
}

function flatTop(n: number, N: number): number {
  const a0 = 0.21557895
  const a1 = 0.41663158
  const a2 = 0.277263158
  const a3 = 0.083578947
  const a4 = 0.006947368
  const t = (2 * Math.PI * n) / (N - 1)
  return a0 - a1 * Math.cos(t) + a2 * Math.cos(2 * t) - a3 * Math.cos(3 * t) + a4 * Math.cos(4 * t)
}

function buildWindow(type: WindowType, N: number): Float32Array {
  const w = new Float32Array(N)
  const fn = type === 'hanning' ? hanning : type === 'blackman-harris' ? blackmanHarris : flatTop
  for (let i = 0; i < N; i++) w[i] = fn(i, N)
  return w
}

function windowCoherentGain(w: Float32Array): number {
  let sum = 0
  for (let i = 0; i < w.length; i++) sum += w[i]
  return sum / w.length
}

function fftRadix2(re: Float32Array, im: Float32Array): void {
  const n = re.length
  let j = 0
  for (let i = 1; i < n; i++) {
    let bit = n >> 1
    for (; j & bit; bit >>= 1) j ^= bit
    j ^= bit
    if (i < j) {
      const tr = re[i]; re[i] = re[j]; re[j] = tr
      const ti = im[i]; im[i] = im[j]; im[j] = ti
    }
  }
  for (let len = 2; len <= n; len <<= 1) {
    const half = len >> 1
    const ang = (-2 * Math.PI) / len
    const wre = Math.cos(ang)
    const wim = Math.sin(ang)
    for (let i = 0; i < n; i += len) {
      let cur_re = 1
      let cur_im = 0
      for (let k = 0; k < half; k++) {
        const a_re = re[i + k]
        const a_im = im[i + k]
        const b_re = re[i + k + half] * cur_re - im[i + k + half] * cur_im
        const b_im = re[i + k + half] * cur_im + im[i + k + half] * cur_re
        re[i + k] = a_re + b_re
        im[i + k] = a_im + b_im
        re[i + k + half] = a_re - b_re
        im[i + k + half] = a_im - b_im
        const nr = cur_re * wre - cur_im * wim
        cur_im = cur_re * wim + cur_im * wre
        cur_re = nr
      }
    }
  }
}

function computeMagnitudesDb(
  samples: Float32Array,
  window: Float32Array,
  coherentGain: number,
): Float32Array {
  const N = samples.length
  const re = new Float32Array(N)
  const im = new Float32Array(N)
  for (let i = 0; i < N; i++) re[i] = samples[i] * window[i]
  fftRadix2(re, im)
  const bins = N / 2
  const out = new Float32Array(bins)
  const scale = 1 / (N * coherentGain)
  for (let k = 0; k < bins; k++) {
    const mag = Math.sqrt(re[k] * re[k] + im[k] * im[k]) * 2 * scale
    out[k] = mag > 1e-10 ? 20 * Math.log10(mag) : MIN_DB
  }
  return out
}

function freqToX(freq: number, sampleRate: number, scale: FreqScale, w: number): number {
  const fMin = 20
  const fMax = sampleRate / 2
  if (scale === 'log') {
    const logMin = Math.log10(fMin)
    const logMax = Math.log10(fMax)
    return ((Math.log10(Math.max(freq, fMin)) - logMin) / (logMax - logMin)) * w
  }
  return (freq / fMax) * w
}

function xToFreq(x: number, sampleRate: number, scale: FreqScale, w: number): number {
  const fMin = 20
  const fMax = sampleRate / 2
  if (scale === 'log') {
    const logMin = Math.log10(fMin)
    const logMax = Math.log10(fMax)
    return Math.pow(10, logMin + (x / w) * (logMax - logMin))
  }
  return (x / w) * fMax
}

export function SpectrumAnalyzer({ onClose }: SpectrumAnalyzerProps) {
  const [fftSize, setFftSize] = useState<FftSize>(4096)
  const [windowType, setWindowType] = useState<WindowType>('hanning')
  const [displayStyle, setDisplayStyle] = useState<DisplayStyle>('line')
  const [freqScale, setFreqScale] = useState<FreqScale>('log')
  const [channelMode, setChannelMode] = useState<ChannelMode>('lr')
  const [peakHold, setPeakHold] = useState(true)
  const [smoothing, setSmoothing] = useState(0.5)
  const [freeze, setFreeze] = useState(false)
  const [sampleRate, setSampleRate] = useState(48000)
  const [hover, setHover] = useState<{ x: number; y: number } | null>(null)

  const canvasRef = useRef<HTMLCanvasElement>(null)
  const leftDbRef = useRef<Float32Array>(new Float32Array(0))
  const rightDbRef = useRef<Float32Array>(new Float32Array(0))
  const leftPeakRef = useRef<Float32Array>(new Float32Array(0))
  const rightPeakRef = useRef<Float32Array>(new Float32Array(0))

  const fftWindow = useMemo(() => buildWindow(windowType, fftSize), [windowType, fftSize])
  const coherentGain = useMemo(() => windowCoherentGain(fftWindow), [fftWindow])

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
    leftDbRef.current = new Float32Array(fftSize / 2)
    rightDbRef.current = new Float32Array(fftSize / 2)
    leftPeakRef.current = new Float32Array(fftSize / 2).fill(MIN_DB)
    rightPeakRef.current = new Float32Array(fftSize / 2).fill(MIN_DB)
    for (let i = 0; i < fftSize / 2; i++) {
      leftDbRef.current[i] = MIN_DB
      rightDbRef.current[i] = MIN_DB
    }
  }, [fftSize])

  useEffect(() => {
    if (freeze) return
    const id = setInterval(async () => {
      try {
        const samples = await invoke<number[]>('get_master_samples', { nFrames: fftSize })
        const n = Math.floor(samples.length / 2)
        if (n < fftSize) return
        const l = new Float32Array(fftSize)
        const r = new Float32Array(fftSize)
        if (channelMode === 'lr') {
          for (let i = 0; i < fftSize; i++) {
            l[i] = samples[i * 2]
            r[i] = samples[i * 2 + 1]
          }
        } else {
          for (let i = 0; i < fftSize; i++) {
            const ls = samples[i * 2]
            const rs = samples[i * 2 + 1]
            l[i] = (ls + rs) * 0.5
            r[i] = (ls - rs) * 0.5
          }
        }
        const newL = computeMagnitudesDb(l, fftWindow, coherentGain)
        const newR = computeMagnitudesDb(r, fftWindow, coherentGain)

        const prevL = leftDbRef.current
        const prevR = rightDbRef.current
        const s = smoothing
        for (let i = 0; i < newL.length; i++) {
          prevL[i] = prevL[i] * s + newL[i] * (1 - s)
          prevR[i] = prevR[i] * s + newR[i] * (1 - s)
        }

        if (peakHold) {
          const pkL = leftPeakRef.current
          const pkR = rightPeakRef.current
          for (let i = 0; i < prevL.length; i++) {
            if (prevL[i] > pkL[i]) pkL[i] = prevL[i]
            else pkL[i] -= 0.5
            if (prevR[i] > pkR[i]) pkR[i] = prevR[i]
            else pkR[i] -= 0.5
          }
        }

        const c = canvasRef.current
        if (c) draw(c, prevL, prevR, leftPeakRef.current, rightPeakRef.current,
          sampleRate, freqScale, displayStyle, peakHold, channelMode, hover)
      } catch { /* ignore */ }
    }, POLL_INTERVAL_MS)
    return () => clearInterval(id)
  }, [fftSize, fftWindow, coherentGain, channelMode, smoothing, peakHold, freeze,
      sampleRate, freqScale, displayStyle, hover])

  const resetPeaks = () => {
    leftPeakRef.current.fill(MIN_DB)
    rightPeakRef.current.fill(MIN_DB)
  }

  const hoverInfo = useMemo(() => {
    if (!hover) return null
    const w = 680
    const freq = xToFreq(hover.x, sampleRate, freqScale, w)
    const h = 320
    const db = MAX_DB - (hover.y / h) * (MAX_DB - MIN_DB)
    return { freq, db }
  }, [hover, sampleRate, freqScale])

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
          <div style={{ fontSize: 12, fontWeight: 600 }}>Spectrum Analyzer</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>
            Master bus · {sampleRate} Hz · {fftSize}-point FFT
          </div>
          <div style={{ flex: 1 }} />
          <button onClick={resetPeaks} style={btnStyle(false)}>Reset peaks</button>
          <button onClick={() => setFreeze(v => !v)} style={btnStyle(freeze)}>
            {freeze ? 'Frozen' : 'Freeze'}
          </button>
          <button onClick={onClose} style={btnStyle(false)}>Close</button>
        </div>

        <canvas
          ref={canvasRef}
          width={680}
          height={320}
          style={{ width: '100%', height: 320, background: '#0a0a0e', display: 'block', cursor: 'crosshair' }}
          onMouseMove={(e) => {
            const rect = e.currentTarget.getBoundingClientRect()
            setHover({ x: e.clientX - rect.left, y: e.clientY - rect.top })
          }}
          onMouseLeave={() => setHover(null)}
        />

        {hoverInfo && (
          <div style={{
            padding: '4px 12px', fontSize: 10, color: hw.textSecondary,
            background: hw.bgElevated, borderTop: `1px solid ${hw.border}`,
            fontVariantNumeric: 'tabular-nums',
          }}>
            {hoverInfo.freq >= 1000
              ? `${(hoverInfo.freq / 1000).toFixed(2)} kHz`
              : `${hoverInfo.freq.toFixed(1)} Hz`}
            &nbsp;· {hoverInfo.db.toFixed(1)} dB
          </div>
        )}

        <div style={{
          padding: '8px 12px', borderTop: `1px solid ${hw.border}`,
          background: hw.bgElevated, display: 'flex', alignItems: 'center',
          gap: 12, flexWrap: 'wrap',
        }}>
          <LabeledSelect label="FFT" value={String(fftSize)}
            onChange={v => setFftSize(parseInt(v, 10) as FftSize)}
            options={[['1024','1024'],['2048','2048'],['4096','4096'],['8192','8192']]} />

          <LabeledSelect label="Window" value={windowType}
            onChange={v => setWindowType(v as WindowType)}
            options={[['hanning','Hanning'],['blackman-harris','Blackman-Harris'],['flat-top','Flat-top']]} />

          <LabeledSelect label="Style" value={displayStyle}
            onChange={v => setDisplayStyle(v as DisplayStyle)}
            options={[['line','Line'],['bars','Bars']]} />

          <LabeledSelect label="Scale" value={freqScale}
            onChange={v => setFreqScale(v as FreqScale)}
            options={[['log','Log'],['linear','Linear']]} />

          <LabeledSelect label="Channels" value={channelMode}
            onChange={v => setChannelMode(v as ChannelMode)}
            options={[['lr','L / R'],['mid-side','Mid / Side']]} />

          <label style={labelStyle}>
            <span>Smooth</span>
            <input type="range" min={0} max={0.95} step={0.05} value={smoothing}
              onChange={e => setSmoothing(parseFloat(e.target.value))}
              style={{ width: 80 }} />
            <span style={{ color: hw.textPrimary, fontVariantNumeric: 'tabular-nums', minWidth: 28 }}>
              {smoothing.toFixed(2)}
            </span>
          </label>

          <label style={labelStyle}>
            <input type="checkbox" checked={peakHold} onChange={e => setPeakHold(e.target.checked)} />
            <span>Peak hold</span>
          </label>
        </div>
      </div>
    </div>
  )
}

const labelStyle = {
  fontSize: 10, color: hw.textSecondary,
  display: 'flex', gap: 6, alignItems: 'center',
} as const

function btnStyle(active: boolean) {
  return {
    padding: '3px 10px', fontSize: 10, background: 'transparent',
    border: `1px solid ${active ? hw.accent : hw.border}`, borderRadius: hw.radius.sm,
    color: active ? hw.accent : hw.textSecondary, cursor: 'pointer',
    transition: 'color 0.15s, border-color 0.15s',
  } as const
}

function LabeledSelect({ label, value, onChange, options }: {
  label: string; value: string; onChange: (v: string) => void; options: [string, string][];
}) {
  return (
    <label style={labelStyle}>
      <span>{label}</span>
      <select value={value} onChange={e => onChange(e.target.value)} style={{
        fontSize: 10, padding: '2px 6px', background: hw.bg,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
        color: hw.textPrimary,
      }}>
        {options.map(([v, n]) => <option key={v} value={v}>{n}</option>)}
      </select>
    </label>
  )
}

function draw(
  c: HTMLCanvasElement,
  leftDb: Float32Array,
  rightDb: Float32Array,
  leftPeak: Float32Array,
  rightPeak: Float32Array,
  sampleRate: number,
  freqScale: FreqScale,
  displayStyle: DisplayStyle,
  peakHold: boolean,
  channelMode: ChannelMode,
  hover: { x: number; y: number } | null,
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

  drawGrid(ctx, w, h, sampleRate, freqScale)

  const colorL = channelMode === 'mid-side' ? hw.accent : hw.accent
  const colorR = channelMode === 'mid-side' ? hw.secondary : hw.secondary

  drawSpectrum(ctx, w, h, leftDb, sampleRate, freqScale, displayStyle, colorL)
  drawSpectrum(ctx, w, h, rightDb, sampleRate, freqScale, displayStyle, colorR)

  if (peakHold) {
    drawPeakLine(ctx, w, h, leftPeak, sampleRate, freqScale, colorL)
    drawPeakLine(ctx, w, h, rightPeak, sampleRate, freqScale, colorR)
  }

  if (hover) {
    ctx.strokeStyle = hw.textFaint
    ctx.lineWidth = 1
    ctx.setLineDash([3, 3])
    ctx.beginPath()
    ctx.moveTo(hover.x, 0)
    ctx.lineTo(hover.x, h)
    ctx.moveTo(0, hover.y)
    ctx.lineTo(w, hover.y)
    ctx.stroke()
    ctx.setLineDash([])
  }
}

function drawGrid(
  ctx: CanvasRenderingContext2D, w: number, h: number,
  sampleRate: number, freqScale: FreqScale,
) {
  ctx.strokeStyle = hw.border
  ctx.lineWidth = 1
  ctx.font = '9px sans-serif'
  ctx.fillStyle = hw.textFaint

  for (let db = 0; db >= MIN_DB; db -= 20) {
    const y = ((MAX_DB - db) / (MAX_DB - MIN_DB)) * h
    ctx.beginPath()
    ctx.moveTo(0, y)
    ctx.lineTo(w, y)
    ctx.stroke()
    ctx.fillText(`${db}`, 2, y - 2)
  }

  const gridFreqs = freqScale === 'log'
    ? [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000]
    : [2000, 4000, 6000, 8000, 10000, 12000, 14000, 16000, 18000, 20000]
  for (const f of gridFreqs) {
    if (f > sampleRate / 2) continue
    const x = freqToX(f, sampleRate, freqScale, w)
    ctx.beginPath()
    ctx.moveTo(x, 0)
    ctx.lineTo(x, h)
    ctx.stroke()
    const label = f >= 1000 ? `${f / 1000}k` : `${f}`
    ctx.fillText(label, x + 2, h - 2)
  }
}

function drawSpectrum(
  ctx: CanvasRenderingContext2D, w: number, h: number,
  db: Float32Array, sampleRate: number, freqScale: FreqScale,
  displayStyle: DisplayStyle, color: string,
) {
  const nyquist = sampleRate / 2
  const binHz = nyquist / db.length
  ctx.strokeStyle = color
  ctx.fillStyle = color + '44'
  ctx.lineWidth = 1.25

  if (displayStyle === 'bars') {
    const bars = 128
    for (let b = 0; b < bars; b++) {
      const f0 = freqScale === 'log'
        ? Math.pow(10, Math.log10(20) + (b / bars) * (Math.log10(nyquist) - Math.log10(20)))
        : (b / bars) * nyquist
      const f1 = freqScale === 'log'
        ? Math.pow(10, Math.log10(20) + ((b + 1) / bars) * (Math.log10(nyquist) - Math.log10(20)))
        : ((b + 1) / bars) * nyquist
      const i0 = Math.max(0, Math.floor(f0 / binHz))
      const i1 = Math.min(db.length - 1, Math.ceil(f1 / binHz))
      let maxDb = MIN_DB
      for (let i = i0; i <= i1; i++) if (db[i] > maxDb) maxDb = db[i]
      const x0 = freqToX(f0, sampleRate, freqScale, w)
      const x1 = freqToX(f1, sampleRate, freqScale, w)
      const y = ((MAX_DB - Math.max(MIN_DB, Math.min(MAX_DB, maxDb))) / (MAX_DB - MIN_DB)) * h
      ctx.fillRect(x0, y, Math.max(1, x1 - x0 - 1), h - y)
    }
    return
  }

  ctx.beginPath()
  let started = false
  for (let i = 1; i < db.length; i++) {
    const freq = i * binHz
    if (freq < 20) continue
    const x = freqToX(freq, sampleRate, freqScale, w)
    const v = Math.max(MIN_DB, Math.min(MAX_DB, db[i]))
    const y = ((MAX_DB - v) / (MAX_DB - MIN_DB)) * h
    if (!started) { ctx.moveTo(x, y); started = true } else ctx.lineTo(x, y)
  }
  ctx.stroke()
}

function drawPeakLine(
  ctx: CanvasRenderingContext2D, w: number, h: number,
  peak: Float32Array, sampleRate: number, freqScale: FreqScale, color: string,
) {
  const nyquist = sampleRate / 2
  const binHz = nyquist / peak.length
  ctx.strokeStyle = color + '88'
  ctx.lineWidth = 1
  ctx.beginPath()
  let started = false
  for (let i = 1; i < peak.length; i++) {
    const freq = i * binHz
    if (freq < 20) continue
    const x = freqToX(freq, sampleRate, freqScale, w)
    const v = Math.max(MIN_DB, Math.min(MAX_DB, peak[i]))
    const y = ((MAX_DB - v) / (MAX_DB - MIN_DB)) * h
    if (!started) { ctx.moveTo(x, y); started = true } else ctx.lineTo(x, y)
  }
  ctx.stroke()
}
