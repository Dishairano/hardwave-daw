import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { hw } from '../../theme'
import { encodeWav } from '../../utils/wav'

interface BeatSlicerProps {
  path: string
  onClose: () => void
}

interface Sample {
  channels: Float32Array[]
  sampleRate: number
}

interface Slice {
  start: number
  adsr: { a: number; d: number; s: number; r: number }
  vol: number
  pan: number
  reverse: boolean
}

const KEYMAP = ['a','s','d','f','g','h','j','k','l',';',"'",'\\']

function cloneSlice(s: Slice): Slice {
  return { start: s.start, adsr: { ...s.adsr }, vol: s.vol, pan: s.pan, reverse: s.reverse }
}

function defaultSlice(start: number): Slice {
  return { start, adsr: { a: 0.005, d: 0.05, s: 1, r: 0.02 }, vol: 1, pan: 0, reverse: false }
}

function detectTransients(ch: Float32Array, sampleRate: number, threshold: number): number[] {
  const win = Math.max(256, Math.floor(sampleRate * 0.010))
  const hop = Math.floor(win / 2)
  const energies: number[] = []
  for (let i = 0; i + win < ch.length; i += hop) {
    let sum = 0
    for (let j = 0; j < win; j++) {
      const v = ch[i + j]
      sum += v * v
    }
    energies.push(Math.sqrt(sum / win))
  }
  const flux: number[] = [0]
  for (let i = 1; i < energies.length; i++) {
    flux.push(Math.max(0, energies[i] - energies[i - 1]))
  }
  let maxFlux = 0
  for (const v of flux) if (v > maxFlux) maxFlux = v
  if (maxFlux < 1e-6) return []
  const minGap = Math.floor(sampleRate * 0.040 / hop)
  const out: number[] = []
  let lastPick = -minGap
  for (let i = 0; i < flux.length; i++) {
    if (flux[i] / maxFlux >= threshold && i - lastPick >= minGap) {
      out.push(i * hop)
      lastPick = i
    }
  }
  return out
}

function sliceEnd(slices: Slice[], i: number, total: number) {
  if (i + 1 < slices.length) return slices[i + 1].start
  return total
}

export function BeatSlicer({ path, onClose }: BeatSlicerProps) {
  const [sample, setSample] = useState<Sample | null>(null)
  const [loading, setLoading] = useState(true)
  const [err, setErr] = useState<string | null>(null)
  const [slices, setSlices] = useState<Slice[]>([])
  const [selectedSlice, setSelectedSlice] = useState<number | null>(null)
  const [threshold, setThreshold] = useState(0.25)
  const [banner, setBanner] = useState<string | null>(null)
  const [sliceView, setSliceView] = useState<'list' | 'pads'>('list')

  const canvasRef = useRef<HTMLCanvasElement>(null)
  const ctxRef = useRef<AudioContext | null>(null)

  const name = path.split(/[\\/]/).pop() || path

  useEffect(() => {
    let cancelled = false
    ;(async () => {
      setLoading(true)
      setErr(null)
      try {
        const { convertFileSrc } = await import('@tauri-apps/api/core')
        const url = convertFileSrc(path)
        const resp = await fetch(url)
        if (!resp.ok) throw new Error('Failed to fetch audio file')
        const buf = await resp.arrayBuffer()
        const AC: typeof AudioContext = (window as unknown as { AudioContext: typeof AudioContext; webkitAudioContext?: typeof AudioContext }).AudioContext
          || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext
        const ctx = new AC()
        const decoded = await ctx.decodeAudioData(buf.slice(0))
        if (cancelled) return
        const channels: Float32Array[] = []
        for (let i = 0; i < decoded.numberOfChannels; i++) {
          channels.push(new Float32Array(decoded.getChannelData(i)))
        }
        setSample({ channels, sampleRate: decoded.sampleRate })
        ctx.close().catch(() => {})
      } catch (e) {
        if (!cancelled) setErr(String(e))
      }
      if (!cancelled) setLoading(false)
    })()
    return () => { cancelled = true }
  }, [path])

  useEffect(() => {
    return () => {
      ctxRef.current?.close().catch(() => {})
    }
  }, [])

  const runAutoDetect = useCallback(() => {
    if (!sample) return
    const idx = detectTransients(sample.channels[0], sample.sampleRate, threshold)
    const newSlices: Slice[] = idx.length === 0 ? [defaultSlice(0)] : idx.map(defaultSlice)
    if (newSlices[0].start !== 0) newSlices.unshift(defaultSlice(0))
    setSlices(newSlices)
    setSelectedSlice(0)
    setBanner(`Detected ${newSlices.length} slice${newSlices.length === 1 ? '' : 's'}`)
  }, [sample, threshold])

  useEffect(() => {
    if (sample && slices.length === 0) runAutoDetect()
  }, [sample, slices.length, runAutoDetect])

  useEffect(() => {
    if (!sample || !canvasRef.current) return
    drawWaveform(canvasRef.current, sample, slices, selectedSlice)
  }, [sample, slices, selectedSlice])

  useEffect(() => {
    if (!banner) return
    const t = setTimeout(() => setBanner(null), 2000)
    return () => clearTimeout(t)
  }, [banner])

  const playSlice = useCallback((i: number) => {
    if (!sample) return
    const s = slices[i]
    if (!s) return
    const end = sliceEnd(slices, i, sample.channels[0].length)
    const start = s.start
    const length = Math.max(1, end - start)
    const AC: typeof AudioContext = (window as unknown as { AudioContext: typeof AudioContext; webkitAudioContext?: typeof AudioContext }).AudioContext
      || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext
    const ctx = ctxRef.current ?? new AC({ sampleRate: sample.sampleRate })
    ctxRef.current = ctx
    const buf = ctx.createBuffer(sample.channels.length, length, sample.sampleRate)
    for (let c = 0; c < sample.channels.length; c++) {
      const out = buf.getChannelData(c)
      if (s.reverse) {
        for (let j = 0; j < length; j++) out[j] = sample.channels[c][end - 1 - j] ?? 0
      } else {
        out.set(sample.channels[c].subarray(start, end))
      }
    }
    const src = ctx.createBufferSource()
    src.buffer = buf

    const gain = ctx.createGain()
    const panner = ctx.createStereoPanner()
    panner.pan.value = Math.max(-1, Math.min(1, s.pan))

    const now = ctx.currentTime
    const peak = Math.max(0, s.vol)
    const a = Math.max(0, s.adsr.a)
    const d = Math.max(0, s.adsr.d)
    const sus = Math.max(0, Math.min(1, s.adsr.s)) * peak
    const r = Math.max(0.001, s.adsr.r)
    const dur = length / sample.sampleRate

    gain.gain.setValueAtTime(0, now)
    gain.gain.linearRampToValueAtTime(peak, now + a)
    gain.gain.linearRampToValueAtTime(sus, now + a + d)
    const relStart = Math.max(now + a + d, now + Math.max(0, dur - r))
    gain.gain.setValueAtTime(gain.gain.value, relStart)
    gain.gain.linearRampToValueAtTime(0, relStart + r)

    src.connect(gain).connect(panner).connect(ctx.destination)
    src.start(now)
    src.stop(now + dur + r + 0.01)
  }, [sample, slices])

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement)?.tagName === 'INPUT') return
      if (e.ctrlKey || e.metaKey || e.altKey) return
      const idx = KEYMAP.indexOf(e.key.toLowerCase())
      if (idx >= 0 && idx < slices.length) {
        e.preventDefault()
        playSlice(idx)
        setSelectedSlice(idx)
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [slices, playSlice])

  const onCanvasClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!sample || !canvasRef.current) return
    const rect = canvasRef.current.getBoundingClientRect()
    const x = e.clientX - rect.left
    const frac = x / rect.width
    const total = sample.channels[0].length
    const sampleIdx = Math.floor(frac * total)
    if (e.shiftKey) {
      const hitRadius = Math.floor(total * 0.005)
      const hit = slices.findIndex(s => Math.abs(s.start - sampleIdx) <= hitRadius)
      if (hit > 0) {
        const next = slices.filter((_, i) => i !== hit)
        setSlices(next)
        setSelectedSlice(null)
        setBanner('Slice removed')
      }
      return
    }
    const hitRadius = Math.floor(total * 0.005)
    const hit = slices.findIndex(s => Math.abs(s.start - sampleIdx) <= hitRadius)
    if (hit >= 0) {
      setSelectedSlice(hit)
      playSlice(hit)
      return
    }
    const next = [...slices, defaultSlice(sampleIdx)].sort((a, b) => a.start - b.start)
    setSlices(next)
    const newIdx = next.findIndex(s => s.start === sampleIdx)
    setSelectedSlice(newIdx)
    setBanner('Slice added')
  }

  const updateSlice = (idx: number, patch: Partial<Slice>) => {
    setSlices(prev => prev.map((s, i) => i === idx ? { ...cloneSlice(s), ...patch } : s))
  }

  const updateAdsr = (idx: number, patch: Partial<Slice['adsr']>) => {
    setSlices(prev => prev.map((s, i) => i === idx ? { ...cloneSlice(s), adsr: { ...s.adsr, ...patch } } : s))
  }

  const clearAll = () => { setSlices([defaultSlice(0)]); setSelectedSlice(0) }

  const exportSlices = () => {
    if (!sample || slices.length === 0) return
    const base = name.replace(/\.[^.]+$/, '')
    slices.forEach((s, i) => {
      const end = sliceEnd(slices, i, sample.channels[0].length)
      const length = end - s.start
      if (length <= 0) return
      const channels = sample.channels.map(ch => {
        const out = new Float32Array(length)
        if (s.reverse) {
          for (let j = 0; j < length; j++) out[j] = ch[end - 1 - j] ?? 0
        } else {
          out.set(ch.subarray(s.start, end))
        }
        const vol = s.vol
        const a = Math.max(1, Math.floor(s.adsr.a * sample.sampleRate))
        const r = Math.max(1, Math.floor(s.adsr.r * sample.sampleRate))
        for (let j = 0; j < Math.min(a, length); j++) out[j] *= (j / a) * vol
        for (let j = a; j < length - r; j++) out[j] *= vol
        for (let j = 0; j < r && length - 1 - j >= 0; j++) {
          out[length - 1 - j] *= (j / r) * vol
        }
        return out
      })
      const blob = encodeWav(channels, sample.sampleRate)
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = `${base}_slice${String(i + 1).padStart(2, '0')}.wav`
      document.body.appendChild(a)
      a.click()
      document.body.removeChild(a)
      setTimeout(() => URL.revokeObjectURL(url), 1000)
    })
    setBanner(`Exported ${slices.length} slices`)
  }

  const sliceDurationSec = useMemo(() => {
    if (!sample || selectedSlice === null) return 0
    const s = slices[selectedSlice]
    if (!s) return 0
    const end = sliceEnd(slices, selectedSlice, sample.channels[0].length)
    return (end - s.start) / sample.sampleRate
  }, [sample, slices, selectedSlice])

  const sel = selectedSlice !== null ? slices[selectedSlice] : null

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 10500,
        background: 'rgba(0,0,0,0.55)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div
        style={{
          width: 960, maxWidth: '96vw', maxHeight: '92vh',
          background: hw.bg, border: `1px solid ${hw.border}`,
          borderRadius: hw.radius.lg, display: 'flex', flexDirection: 'column',
          overflow: 'hidden', color: hw.textPrimary,
        }}
      >
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center',
          gap: 12, borderBottom: `1px solid ${hw.border}`, background: hw.bgElevated,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Beat Slicer</div>
          <div style={{ fontSize: 10, color: hw.textFaint, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1 }}>
            {name}
          </div>
          <button onClick={onClose} style={btnStyle('ghost')}>Close</button>
        </div>

        {loading && <div style={{ padding: 24, fontSize: 12, color: hw.textSecondary }}>Loading…</div>}
        {err && <div style={{ padding: 24, fontSize: 11, color: hw.red }}>Error: {err}</div>}

        {sample && !loading && !err && (
          <>
            <canvas
              ref={canvasRef}
              width={960}
              height={220}
              onClick={onCanvasClick}
              style={{ width: '100%', height: 220, background: '#0a0a0e', cursor: 'crosshair', display: 'block' }}
            />
            <div style={{
              padding: '6px 12px', display: 'flex', gap: 10, alignItems: 'center',
              borderTop: `1px solid ${hw.border}`, borderBottom: `1px solid ${hw.border}`,
              background: hw.bgElevated, flexWrap: 'wrap',
            }}>
              <div style={{ fontSize: 10, color: hw.textFaint }}>
                Click to add · Shift+click to remove · Keys {KEYMAP.slice(0, Math.min(KEYMAP.length, slices.length)).join(' ')} play slices
              </div>
              <div style={{ flex: 1 }} />
              <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 10 }}>
                <span style={{ color: hw.textFaint }}>Threshold</span>
                <input
                  type="range" min={0.02} max={0.8} step={0.01} value={threshold}
                  onChange={(e) => setThreshold(parseFloat(e.target.value))}
                  style={{ width: 120 }}
                />
                <span style={{ width: 28, textAlign: 'right' }}>{threshold.toFixed(2)}</span>
              </label>
              <button onClick={runAutoDetect} style={btnStyle('primary')}>Auto-detect</button>
              <button onClick={clearAll} style={btnStyle('ghost')}>Clear</button>
              <div style={{ display: 'flex', gap: 0, border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, overflow: 'hidden' }}>
                <button
                  onClick={() => setSliceView('list')}
                  style={viewToggleStyle(sliceView === 'list')}
                  title="Show slices as a vertical list"
                >List</button>
                <button
                  onClick={() => setSliceView('pads')}
                  style={viewToggleStyle(sliceView === 'pads')}
                  title="Show slices as a drum pad grid"
                >Pads</button>
              </div>
              <button onClick={exportSlices} style={btnStyle('accent')}>Export WAVs</button>
            </div>

            <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
              <div style={{
                flex: '0 0 240px', borderRight: `1px solid ${hw.border}`,
                overflowY: 'auto', padding: 6,
              }}>
                {slices.length === 0 && (
                  <div style={{ padding: 12, fontSize: 10, color: hw.textFaint }}>
                    No slices yet. Click the waveform to add one.
                  </div>
                )}
                {sliceView === 'list' && slices.map((s, i) => {
                  const dur = ((sliceEnd(slices, i, sample.channels[0].length) - s.start) / sample.sampleRate)
                  const active = i === selectedSlice
                  return (
                    <button
                      key={i}
                      onClick={() => { setSelectedSlice(i); playSlice(i) }}
                      style={{
                        width: '100%', display: 'flex', alignItems: 'center', gap: 8,
                        padding: '5px 8px', marginBottom: 2,
                        background: active ? hw.selectionDim : 'transparent',
                        border: `1px solid ${active ? hw.accent : 'transparent'}`,
                        borderRadius: hw.radius.sm, fontSize: 10,
                        color: active ? hw.textPrimary : hw.textSecondary,
                        cursor: 'pointer', textAlign: 'left',
                      }}
                    >
                      <span style={{ width: 14, textAlign: 'center', color: hw.accent, fontFamily: 'monospace' }}>
                        {KEYMAP[i] ?? '·'}
                      </span>
                      <span style={{ flex: 1 }}>Slice {i + 1}</span>
                      <span style={{ color: hw.textFaint, fontVariantNumeric: 'tabular-nums' }}>
                        {dur.toFixed(3)}s
                      </span>
                    </button>
                  )
                })}
                {sliceView === 'pads' && (
                  <PadGrid
                    slices={slices}
                    selected={selectedSlice}
                    keymap={KEYMAP}
                    onTrigger={(i) => { setSelectedSlice(i); playSlice(i) }}
                  />
                )}
              </div>

              <div style={{ flex: 1, padding: 12, overflowY: 'auto' }}>
                {sel && selectedSlice !== null ? (
                  <>
                    <div style={{ fontSize: 11, fontWeight: 600, marginBottom: 8 }}>
                      Slice {selectedSlice + 1} — {sliceDurationSec.toFixed(3)}s
                    </div>
                    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
                      <Section title="Envelope (ADSR)">
                        <Knob label="A" value={sel.adsr.a} min={0} max={0.5} step={0.001} unit="s"
                          onChange={(v) => updateAdsr(selectedSlice, { a: v })} />
                        <Knob label="D" value={sel.adsr.d} min={0} max={1} step={0.001} unit="s"
                          onChange={(v) => updateAdsr(selectedSlice, { d: v })} />
                        <Knob label="S" value={sel.adsr.s} min={0} max={1} step={0.01}
                          onChange={(v) => updateAdsr(selectedSlice, { s: v })} />
                        <Knob label="R" value={sel.adsr.r} min={0.001} max={1} step={0.001} unit="s"
                          onChange={(v) => updateAdsr(selectedSlice, { r: v })} />
                      </Section>
                      <Section title="Dynamics">
                        <Knob label="Volume" value={sel.vol} min={0} max={1.5} step={0.01}
                          onChange={(v) => updateSlice(selectedSlice, { vol: v })} />
                        <Knob label="Pan" value={sel.pan} min={-1} max={1} step={0.01}
                          onChange={(v) => updateSlice(selectedSlice, { pan: v })} />
                        <label style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 10, marginTop: 6 }}>
                          <input
                            type="checkbox" checked={sel.reverse}
                            onChange={(e) => updateSlice(selectedSlice, { reverse: e.target.checked })}
                          />
                          <span>Reverse</span>
                        </label>
                        <button onClick={() => playSlice(selectedSlice)} style={{ ...btnStyle('primary'), marginTop: 10 }}>
                          Audition
                        </button>
                      </Section>
                    </div>
                  </>
                ) : (
                  <div style={{ fontSize: 10, color: hw.textFaint }}>
                    Select a slice from the list to edit its parameters.
                  </div>
                )}
              </div>
            </div>

            {banner && (
              <div style={{
                padding: '6px 12px', background: hw.selectionDim,
                borderTop: `1px solid ${hw.border}`, fontSize: 10, color: hw.textPrimary,
              }}>
                {banner}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{
      padding: 10, background: hw.bgElevated, border: `1px solid ${hw.border}`,
      borderRadius: hw.radius.md,
    }}>
      <div style={{ fontSize: 9, color: hw.textFaint, textTransform: 'uppercase', letterSpacing: 0.6, marginBottom: 8 }}>
        {title}
      </div>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 10 }}>
        {children}
      </div>
    </div>
  )
}

function Knob({ label, value, min, max, step, unit, onChange }: {
  label: string; value: number; min: number; max: number; step: number; unit?: string;
  onChange: (v: number) => void;
}) {
  return (
    <label style={{ display: 'flex', flexDirection: 'column', gap: 3, fontSize: 9, color: hw.textSecondary, minWidth: 80 }}>
      <span style={{ color: hw.textFaint }}>{label}</span>
      <input type="range" min={min} max={max} step={step} value={value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        style={{ width: '100%' }} />
      <span style={{ fontVariantNumeric: 'tabular-nums', color: hw.textPrimary }}>
        {value.toFixed(3)}{unit ?? ''}
      </span>
    </label>
  )
}

function viewToggleStyle(active: boolean): React.CSSProperties {
  return {
    padding: '4px 10px', fontSize: 10,
    background: active ? hw.accentDim : 'transparent',
    border: 'none',
    color: active ? hw.accent : hw.textSecondary,
    cursor: 'pointer', lineHeight: 1.4,
    fontWeight: active ? 600 : 400,
  }
}

function PadGrid({ slices, selected, keymap, onTrigger }: {
  slices: Slice[]
  selected: number | null
  keymap: string[]
  onTrigger: (i: number) => void
}) {
  if (slices.length === 0) return null
  const cols = slices.length <= 16 ? 4 : Math.ceil(Math.sqrt(slices.length))
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: `repeat(${cols}, 1fr)`,
        gap: 4,
        padding: 2,
      }}
    >
      {slices.map((_, i) => {
        const active = i === selected
        const key = keymap[i]
        return (
          <button
            key={i}
            onClick={() => onTrigger(i)}
            title={`Slice ${i + 1}${key ? ` (${key})` : ''}`}
            style={{
              aspectRatio: '1 / 1',
              display: 'flex', flexDirection: 'column',
              alignItems: 'center', justifyContent: 'center',
              background: active ? hw.accentDim : 'rgba(255,255,255,0.04)',
              border: `1px solid ${active ? hw.accent : hw.border}`,
              borderRadius: hw.radius.sm,
              boxShadow: active ? `0 0 10px ${hw.accentGlow}` : 'none',
              color: active ? hw.textBright : hw.textSecondary,
              cursor: 'pointer', padding: 0,
              fontFamily: 'monospace', gap: 2,
            }}
          >
            <span style={{ fontSize: 11, fontWeight: 700 }}>{i + 1}</span>
            {key && (
              <span style={{ fontSize: 8, color: hw.textFaint, textTransform: 'uppercase' }}>
                {key}
              </span>
            )}
          </button>
        )
      })}
    </div>
  )
}

function btnStyle(variant: 'primary' | 'accent' | 'ghost'): React.CSSProperties {
  const base: React.CSSProperties = {
    padding: '4px 10px', fontSize: 10, borderRadius: hw.radius.sm,
    cursor: 'pointer', border: '1px solid', lineHeight: 1.4,
  }
  if (variant === 'primary') {
    return { ...base, background: hw.bgElevated, borderColor: hw.border, color: hw.textPrimary }
  }
  if (variant === 'accent') {
    return { ...base, background: hw.accent, borderColor: hw.accent, color: '#0a0a0e', fontWeight: 600 }
  }
  return { ...base, background: 'transparent', borderColor: 'transparent', color: hw.textSecondary }
}

function drawWaveform(canvas: HTMLCanvasElement, sample: Sample, slices: Slice[], selected: number | null) {
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  const dpr = window.devicePixelRatio || 1
  const w = canvas.clientWidth
  const h = canvas.clientHeight
  if (canvas.width !== Math.floor(w * dpr) || canvas.height !== Math.floor(h * dpr)) {
    canvas.width = Math.floor(w * dpr)
    canvas.height = Math.floor(h * dpr)
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx.fillStyle = '#0a0a0e'
  ctx.fillRect(0, 0, w, h)

  const ch = sample.channels[0]
  const n = ch.length
  if (n === 0) return
  const midY = h / 2
  const amp = h / 2 - 6

  ctx.strokeStyle = hw.border
  ctx.lineWidth = 1
  ctx.beginPath()
  ctx.moveTo(0, midY)
  ctx.lineTo(w, midY)
  ctx.stroke()

  ctx.strokeStyle = hw.textSecondary
  ctx.lineWidth = 1
  ctx.beginPath()
  const step = n / w
  for (let x = 0; x < w; x++) {
    const s = Math.floor(x * step)
    const e = Math.min(n, Math.floor((x + 1) * step))
    let min = 1, max = -1
    for (let i = s; i < e; i++) {
      const v = ch[i]
      if (v < min) min = v
      if (v > max) max = v
    }
    ctx.moveTo(x, midY - max * amp)
    ctx.lineTo(x, midY - min * amp)
  }
  ctx.stroke()

  slices.forEach((sl, i) => {
    const x = (sl.start / n) * w
    ctx.strokeStyle = i === selected ? hw.accent : hw.secondary
    ctx.lineWidth = i === selected ? 2 : 1
    ctx.beginPath()
    ctx.moveTo(x, 0)
    ctx.lineTo(x, h)
    ctx.stroke()
    ctx.fillStyle = i === selected ? hw.accent : hw.secondary
    ctx.font = '10px monospace'
    ctx.fillText(`${i + 1}`, x + 3, 12)
  })
}
