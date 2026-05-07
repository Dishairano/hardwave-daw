/*
 * KickSynthEditor — per-track patch editor for the native KickSynth.
 *
 * Renders the four hardstyle layers (Transient / Punch / Bass / Tail) as
 * a colour-coded tab strip. Each layer exposes its six parameters
 * (peak gain, length, release, sweep start, sweep end, sweep duration)
 * via circular knobs. Edits flow through the `set_kick_layer` Tauri
 * command and trigger an engine rebuild so the change is audible on
 * the next note-on.
 *
 * Defaults shown when the project's `kick_patch` for that layer is
 * `None` mirror the engine's hard-coded hardstyle preset.
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

const LAYER_NAMES = ['Transient', 'Punch', 'Bass', 'Tail'] as const
const LAYER_CLASSES = ['transient', 'punch', 'bass', 'tail'] as const
type LayerIdx = 0 | 1 | 2 | 3

type Waveform = 'sine' | 'saw' | 'square' | 'triangle'
const WAVEFORMS: Waveform[] = ['sine', 'saw', 'square', 'triangle']

const WAVEFORM_PATHS: Record<Waveform, string> = {
  sine: 'M0,6 Q4,0 8,6 T16,6 T24,6 T32,6',
  saw: 'M0,2 L8,10 L8,2 L16,10 L16,2 L24,10 L24,2 L32,10',
  square: 'M0,2 L0,10 L8,10 L8,2 L16,2 L16,10 L24,10 L24,2 L32,2',
  triangle: 'M0,6 L8,2 L16,10 L24,2 L32,6',
}

interface LayerPatch {
  peak_gain: number
  length_secs: number
  release_secs: number
  sweep_start_hz: number
  sweep_end_hz: number
  sweep_secs: number
  waveform: Waveform
}

const DEFAULT_LAYERS: LayerPatch[] = [
  { peak_gain: 0.30, length_secs: 0.005, release_secs: 0.015, sweep_start_hz: 3000, sweep_end_hz: 600,  sweep_secs: 0.005, waveform: 'sine' },
  { peak_gain: 0.55, length_secs: 0.025, release_secs: 0.040, sweep_start_hz: 220,  sweep_end_hz: 65,   sweep_secs: 0.025, waveform: 'sine' },
  { peak_gain: 0.45, length_secs: 0.180, release_secs: 0.180, sweep_start_hz: 60,   sweep_end_hz: 50,   sweep_secs: 0.180, waveform: 'sine' },
  { peak_gain: 0.30, length_secs: 0.450, release_secs: 0.350, sweep_start_hz: 35,   sweep_end_hz: 32,   sweep_secs: 0.450, waveform: 'sine' },
]

interface Props {
  trackId: string
  patchLayers: (LayerPatch | null)[]
  drive: number
  onClose: () => void
}

export function KickSynthEditor({ trackId, patchLayers, drive, onClose }: Props) {
  const [activeLayer, setActiveLayer] = useState<LayerIdx>(1)
  const [presets, setPresets] = useState<string[]>([])

  useEffect(() => {
    let alive = true
    invoke<string[]>('list_kick_presets').then(p => { if (alive) setPresets(p) }).catch(() => {})
    return () => { alive = false }
  }, [])

  const applyPreset = async (name: string) => {
    if (!name) return
    await invoke('apply_kick_preset', { trackId, presetName: name })
  }

  const effective: LayerPatch[] = useMemo(
    () => DEFAULT_LAYERS.map((d, i) => patchLayers[i] ?? d),
    [patchLayers],
  )
  const layer = effective[activeLayer]

  const writeLayer = async (next: LayerPatch) => {
    await invoke('set_kick_layer', {
      trackId,
      layerIndex: activeLayer,
      peakGain: next.peak_gain,
      lengthSecs: next.length_secs,
      releaseSecs: next.release_secs,
      sweepStartHz: next.sweep_start_hz,
      sweepEndHz: next.sweep_end_hz,
      sweepSecs: next.sweep_secs,
      waveform: next.waveform,
    })
  }

  const onParam = <K extends keyof LayerPatch>(key: K, value: number) => {
    void writeLayer({ ...layer, [key]: value })
  }

  const reset = async () => {
    await invoke('reset_kick_patch', { trackId })
  }

  return (
    <div className="fl-kick-editor v2" onMouseDown={(e) => e.stopPropagation()}>
      <div className="fl-kick-editor-head">
        <div className="brand">
          <span className="glyph" aria-hidden="true">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 2 L4 6 v6 c0 4.5 3.5 8.5 8 10 c4.5-1.5 8-5.5 8-10 V6 z"/>
              <path d="M9 12 l2 2 l4-4"/>
            </svg>
          </span>
          <span className="name">Kick<span>Synth</span></span>
        </div>
        <div className="preset-row" role="listbox">
          {presets.length === 0 && <div className="preset-empty">No presets</div>}
          {presets.slice(0, 4).map(name => (
            <button
              key={name}
              type="button"
              className="preset-card"
              onClick={() => applyPreset(name)}
              title={`Load preset: ${name}`}
            >
              <span className="nm">{name}</span>
              <span className="gen">PRESET</span>
            </button>
          ))}
        </div>
        <div className="head-actions">
          <button type="button" className="head-btn" onClick={reset} title="Reset to engine defaults">↺ Reset</button>
          <button type="button" className="head-btn danger" onClick={onClose} title="Close editor">×</button>
        </div>
      </div>

      <WaveformPreview layers={effective} activeLayer={activeLayer} />

      <div className="fl-kick-editor-tabs v2">
        {LAYER_NAMES.map((n, i) => (
          <button
            key={n}
            type="button"
            className={`layer-tab ${LAYER_CLASSES[i]}${activeLayer === i ? ' on' : ''}`}
            onClick={() => setActiveLayer(i as LayerIdx)}
          >
            <span className="dot" aria-hidden="true" />
            <span className="nm">{n}</span>
          </button>
        ))}
      </div>

      <div className="fl-kick-editor-grid v2">
        <Knob label="Peak"     value={layer.peak_gain}      min={0}     max={1}    unit=""    onChange={v => onParam('peak_gain', v)} />
        <Knob label="Length"   value={layer.length_secs}    min={0.001} max={2}    unit="s"   onChange={v => onParam('length_secs', v)} format="ms" />
        <Knob label="Release"  value={layer.release_secs}   min={0.001} max={2}    unit="s"   onChange={v => onParam('release_secs', v)} format="ms" />
        <Knob label="Sweep ↑"  value={layer.sweep_start_hz} min={20}    max={5000} unit="Hz"  onChange={v => onParam('sweep_start_hz', v)} />
        <Knob label="Sweep ↓"  value={layer.sweep_end_hz}   min={20}    max={5000} unit="Hz"  onChange={v => onParam('sweep_end_hz', v)} colour="purple" />
        <Knob label="Sweep T"  value={layer.sweep_secs}     min={0.001} max={2}    unit="s"   onChange={v => onParam('sweep_secs', v)} format="ms" />
      </div>

      <div className="fl-kick-editor-waves v2">
        <span className="lbl">Wave</span>
        {WAVEFORMS.map(w => (
          <button
            key={w}
            type="button"
            className={`wave-chip${layer.waveform === w ? ' on' : ''}`}
            onClick={() => writeLayer({ ...layer, waveform: w })}
            title={`${w} oscillator on this layer`}
          >
            <svg viewBox="0 0 32 12" aria-hidden="true">
              <path d={WAVEFORM_PATHS[w]} stroke="currentColor" fill="none" strokeWidth="1.5" />
            </svg>
            <span className="nm">{w}</span>
          </button>
        ))}
      </div>

      <div className="fl-kick-editor-global">
        <div className="meter">
          <span className="label">Output</span>
          <div className="bar" />
        </div>
        <BigKnob
          label="Drive"
          value={drive}
          onChange={v => { invoke('set_kick_drive', { trackId, drive: v }).catch(() => {}) }}
        />
      </div>
    </div>
  )
}

function clamp(v: number, lo: number, hi: number) { return Math.max(lo, Math.min(hi, v)) }

interface KnobProps {
  label: string
  value: number
  min: number
  max: number
  unit: string
  onChange: (v: number) => void
  /** Optional accent colour for the ring fill. */
  colour?: 'red' | 'purple'
  /** Display format: 'ms' converts seconds to milliseconds for readability. */
  format?: 'ms'
}

function Knob({ label, value, min, max, unit, onChange, colour, format }: KnobProps) {
  const ringRef = useRef<HTMLDivElement>(null)
  const [dragging, setDragging] = useState(false)
  const startYRef = useRef(0)
  const startValRef = useRef(0)

  // Map value→0..100 percentage for the conic-gradient ring fill.
  const pct = clamp(((value - min) / (max - min)) * 100, 0, 100)

  // Display string — convert to ms for tiny seconds values.
  let display: string
  if (format === 'ms') {
    display = `${Math.round(value * 1000)} ms`
  } else if (unit === 'Hz') {
    display = `${Math.round(value)} Hz`
  } else if (unit === '') {
    display = value.toFixed(2)
  } else {
    display = `${value.toFixed(2)}${unit}`
  }

  const onPointerDown = (e: React.PointerEvent) => {
    e.preventDefault()
    setDragging(true)
    startYRef.current = e.clientY
    startValRef.current = value
    ringRef.current?.setPointerCapture(e.pointerId)
  }

  const onPointerMove = (e: React.PointerEvent) => {
    if (!dragging) return
    const dy = startYRef.current - e.clientY
    // 200 px of vertical drag = full range
    const range = max - min
    const next = clamp(startValRef.current + (dy / 200) * range, min, max)
    onChange(next)
  }

  const onPointerUp = (e: React.PointerEvent) => {
    setDragging(false)
    ringRef.current?.releasePointerCapture(e.pointerId)
  }

  const onWheel = (e: React.WheelEvent) => {
    // Coarse scroll = 5%, with shift = 1%
    const range = max - min
    const step = (e.shiftKey ? 0.01 : 0.05) * range * (e.deltaY < 0 ? 1 : -1)
    onChange(clamp(value + step, min, max))
  }

  const ringColor = colour === 'purple' ? 'var(--purple, #a855f7)' : 'var(--red-bright, #ef4444)'

  return (
    <div className="knob">
      <div
        ref={ringRef}
        className={`ring${dragging ? ' dragging' : ''}`}
        style={{ ['--val' as string]: pct, ['--col' as string]: ringColor }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onWheel={onWheel}
        role="slider"
        aria-label={label}
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuenow={value}
      >
        <span className="val">{display}</span>
      </div>
      <span className="lbl">{label}</span>
    </div>
  )
}

interface BigKnobProps {
  label: string
  value: number
  onChange: (v: number) => void
}

function BigKnob({ label, value, onChange }: BigKnobProps) {
  return (
    <div className="big-knob">
      <Knob label={label} value={value} min={0} max={1} unit="" onChange={onChange} />
    </div>
  )
}

interface WaveformPreviewProps {
  layers: LayerPatch[]
  activeLayer: LayerIdx
}

/**
 * Render a stylised waveform of the current patch. Sums the four
 * layer envelopes onto a single SVG path. Not sample-accurate — meant
 * to give visual feedback that the patch shape changed when the user
 * tweaks knobs.
 */
function WaveformPreview({ layers, activeLayer }: WaveformPreviewProps) {
  const path = useMemo(() => buildPath(layers), [layers])
  const filled = useMemo(() => buildFill(path), [path])

  return (
    <div className="waveform-preview">
      <svg viewBox="0 0 1000 80" preserveAspectRatio="none">
        <defs>
          <linearGradient id={`kick-wf-${activeLayer}`} x1="0" x2="0" y1="0" y2="1">
            <stop offset="0%" stopColor="rgba(220,38,38,0.55)" />
            <stop offset="100%" stopColor="rgba(220,38,38,0)" />
          </linearGradient>
        </defs>
        <path d={path} fill="none" stroke="#ef4444" strokeWidth="1.5" />
        <path d={filled} fill={`url(#kick-wf-${activeLayer})`} />
      </svg>
    </div>
  )
}

/**
 * Build a 1000×80 SVG path approximating the summed amplitude envelope
 * of all four layers across the longest layer's duration.
 */
function buildPath(layers: LayerPatch[]): string {
  const samples = 200
  const totalSecs = Math.max(0.05, ...layers.map(l => l.length_secs + l.release_secs))
  const points: [number, number][] = []
  for (let i = 0; i < samples; i++) {
    const t = (i / (samples - 1)) * totalSecs
    let amp = 0
    for (const l of layers) {
      // Triangular envelope: ramp up over length_secs, decay over release_secs.
      if (t <= l.length_secs) {
        amp += l.peak_gain * (t / Math.max(0.0001, l.length_secs))
      } else if (t <= l.length_secs + l.release_secs) {
        const decay = (t - l.length_secs) / Math.max(0.0001, l.release_secs)
        amp += l.peak_gain * (1 - decay)
      }
    }
    const x = (i / (samples - 1)) * 1000
    const y = 40 - clamp(amp * 30, -30, 30)
    points.push([x, y])
  }
  return points.map((p, i) => `${i === 0 ? 'M' : 'L'}${p[0].toFixed(1)},${p[1].toFixed(1)}`).join(' ')
}

function buildFill(path: string): string {
  return `${path} L1000,80 L0,80 Z`
}
