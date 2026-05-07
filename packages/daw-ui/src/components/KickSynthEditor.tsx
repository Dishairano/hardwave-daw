/*
 * KickSynthEditor — per-track patch editor for the native KickSynth.
 *
 * Renders the four layers (Transient / Punch / Bass / Tail) as a tab
 * strip; each layer exposes its six parameters (peak gain, length,
 * release, sweep start, sweep end, sweep duration). Edits flow
 * through the `set_kick_layer` Tauri command and trigger an engine
 * rebuild so the change is audible on the next note-on.
 *
 * Defaults shown when the project's `kick_patch` for that layer is
 * `None` mirror the engine's hard-coded hardstyle preset.
 */
import { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

const LAYER_NAMES = ['Transient', 'Punch', 'Bass', 'Tail'] as const
type LayerIdx = 0 | 1 | 2 | 3

type Waveform = 'sine' | 'saw' | 'square' | 'triangle'
const WAVEFORMS: Waveform[] = ['sine', 'saw', 'square', 'triangle']

interface LayerPatch {
  peak_gain: number
  length_secs: number
  release_secs: number
  sweep_start_hz: number
  sweep_end_hz: number
  sweep_secs: number
  waveform: Waveform
}

// Engine defaults, matched from `hardstyle_default_layers()` in
// `crates/hardwave-dsp/src/kick_synth.rs`. Keeping them in sync is
// manual but the values rarely move.
const DEFAULT_LAYERS: LayerPatch[] = [
  { peak_gain: 0.30, length_secs: 0.005, release_secs: 0.015, sweep_start_hz: 3000, sweep_end_hz: 600,  sweep_secs: 0.005, waveform: 'sine' },
  { peak_gain: 0.55, length_secs: 0.025, release_secs: 0.040, sweep_start_hz: 220,  sweep_end_hz: 65,   sweep_secs: 0.025, waveform: 'sine' },
  { peak_gain: 0.45, length_secs: 0.180, release_secs: 0.180, sweep_start_hz: 60,   sweep_end_hz: 50,   sweep_secs: 0.180, waveform: 'sine' },
  { peak_gain: 0.30, length_secs: 0.450, release_secs: 0.350, sweep_start_hz: 35,   sweep_end_hz: 32,   sweep_secs: 0.450, waveform: 'sine' },
]

interface Props {
  trackId: string
  /** May be partially populated; missing layers fall back to defaults. */
  patchLayers: (LayerPatch | null)[]
  onClose: () => void
}

export function KickSynthEditor({ trackId, patchLayers, onClose }: Props) {
  const [activeLayer, setActiveLayer] = useState<LayerIdx>(1) // Punch is the most-tweaked
  const [presets, setPresets] = useState<string[]>([])
  // Lazy-load the preset list once so we don't make every editor open
  // hit the engine; the list is static for the lifetime of the app.
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
    // Refetch tracks happens via project store mutation listeners
    // attached elsewhere; we don't need to do it inline.
  }

  const onParam = <K extends keyof LayerPatch>(key: K, value: number) => {
    void writeLayer({ ...layer, [key]: value })
  }

  const reset = async () => {
    await invoke('reset_kick_patch', { trackId })
  }

  return (
    <div className="fl-kick-editor" onMouseDown={(e) => e.stopPropagation()}>
      <div className="fl-kick-editor-head">
        <span className="title">KickSynth</span>
        {presets.length > 0 && (
          <select
            className="preset"
            defaultValue=""
            onChange={(e) => {
              const v = e.target.value
              e.currentTarget.value = ''
              applyPreset(v)
            }}
            title="Load a named preset — overwrites all layers"
          >
            <option value="" disabled>Preset…</option>
            {presets.map(name => <option key={name} value={name}>{name}</option>)}
          </select>
        )}
        <button className="reset" type="button" onClick={reset} title="Reset to engine defaults">↺ Reset</button>
        <button className="close" type="button" onClick={onClose} title="Close editor">×</button>
      </div>
      <div className="fl-kick-editor-tabs">
        {LAYER_NAMES.map((n, i) => (
          <button
            key={n}
            type="button"
            className={`tab${activeLayer === i ? ' on' : ''}`}
            onClick={() => setActiveLayer(i as LayerIdx)}
          >
            {n}
          </button>
        ))}
      </div>
      <div className="fl-kick-editor-grid">
        <Knob label="Peak"      value={layer.peak_gain}      min={0} max={1} step={0.01} unit=""  onChange={(v) => onParam('peak_gain', v)} />
        <Knob label="Length"    value={layer.length_secs}    min={0.001} max={2} step={0.001} unit="s"  onChange={(v) => onParam('length_secs', v)} />
        <Knob label="Release"   value={layer.release_secs}   min={0.001} max={2} step={0.001} unit="s"  onChange={(v) => onParam('release_secs', v)} />
        <Knob label="Sweep ↑"   value={layer.sweep_start_hz} min={20} max={5000} step={1} unit="Hz" onChange={(v) => onParam('sweep_start_hz', v)} />
        <Knob label="Sweep ↓"   value={layer.sweep_end_hz}   min={20} max={5000} step={1} unit="Hz" onChange={(v) => onParam('sweep_end_hz', v)} />
        <Knob label="Sweep T"   value={layer.sweep_secs}     min={0.001} max={2} step={0.001} unit="s"  onChange={(v) => onParam('sweep_secs', v)} />
      </div>
      <div className="fl-kick-editor-waves">
        <span className="lbl">Wave</span>
        {WAVEFORMS.map(w => (
          <button
            key={w}
            type="button"
            className={`wave${layer.waveform === w ? ' on' : ''}`}
            onClick={() => writeLayer({ ...layer, waveform: w })}
            title={`${w} oscillator on this layer`}
          >
            {w}
          </button>
        ))}
      </div>
    </div>
  )
}

function Knob({
  label, value, min, max, step, unit, onChange,
}: {
  label: string
  value: number
  min: number
  max: number
  step: number
  unit: string
  onChange: (v: number) => void
}) {
  return (
    <label className="fl-kick-knob">
      <span className="lbl">{label}</span>
      <input
        type="number"
        value={value}
        min={min}
        max={max}
        step={step}
        onChange={(e) => {
          const n = parseFloat(e.target.value)
          if (Number.isFinite(n)) onChange(n)
        }}
      />
      <span className="unit">{unit}</span>
    </label>
  )
}
