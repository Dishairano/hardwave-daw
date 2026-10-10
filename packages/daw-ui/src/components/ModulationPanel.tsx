import { useCallback, useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useTrackStore } from '../stores/trackStore'
import { useNotificationStore } from '../stores/notificationStore'
import { DialogFrame } from './ui/DialogFrame'
import { btn, sel, th, td } from './ui/dialogStyles'

/**
 * The modulation matrix: a source that keeps running, wired to a knob.
 *
 * Automation draws a shape once and plays it back. A filter that
 * breathes for the whole song needed a lane full of points; here it is
 * one route, and the shape never stops.
 */

type LfoShape = 'Sine' | 'Triangle' | 'Square' | 'SawtoothUp' | 'SawtoothDown' | 'RandomSampleAndHold'

interface ModRoute {
  id: string
  name: string
  enabled: boolean
  source: { Lfo: { shape: LfoShape; rate: { TempoSync?: { num: number; den: number }; Hz?: number }; phase_offset: number } }
  track_id: string
  target: { PluginParam: { slot_id: string; param_id: number } }
  center: number
  depth: number
}

interface ParamInfo {
  id: number
  name: string
  value: number
  min: number
  max: number
}

const SHAPES: { value: string; label: string; stored: LfoShape }[] = [
  { value: 'sine', label: 'Sine', stored: 'Sine' },
  { value: 'triangle', label: 'Triangle', stored: 'Triangle' },
  { value: 'square', label: 'Square', stored: 'Square' },
  { value: 'saw_up', label: 'Saw up', stored: 'SawtoothUp' },
  { value: 'saw_down', label: 'Saw down', stored: 'SawtoothDown' },
  { value: 'random', label: 'Random', stored: 'RandomSampleAndHold' },
]

const RATES: { label: string; num: number; den: number }[] = [
  { label: '4 bars', num: 16, den: 1 },
  { label: '2 bars', num: 8, den: 1 },
  { label: '1 bar', num: 4, den: 1 },
  { label: '1/2', num: 1, den: 2 },
  { label: '1/4', num: 1, den: 4 },
  { label: '1/8', num: 1, den: 8 },
  { label: '1/16', num: 1, den: 16 },
]

function rateLabel(route: ModRoute): string {
  const rate = route.source.Lfo.rate
  if (rate.Hz !== undefined) return `${rate.Hz} Hz`
  const sync = rate.TempoSync
  if (!sync) return 'free'
  const match = RATES.find(r => r.num === sync.num && r.den === sync.den)
  return match ? match.label : `${sync.num}/${sync.den}`
}

function shapeLabel(route: ModRoute): string {
  return SHAPES.find(s => s.stored === route.source.Lfo.shape)?.label ?? route.source.Lfo.shape
}

export function ModulationPanel({ onClose }: { onClose: () => void }) {
  const tracks = useTrackStore(s => s.tracks)
  const [routes, setRoutes] = useState<ModRoute[]>([])
  const [trackId, setTrackId] = useState('')
  const [slotId, setSlotId] = useState('')
  const [paramId, setParamId] = useState(-1)
  const [params, setParams] = useState<ParamInfo[]>([])
  const [shape, setShape] = useState('sine')
  const [rateIndex, setRateIndex] = useState(4)
  const [depth, setDepth] = useState(0.3)

  const refresh = useCallback(async () => {
    try {
      setRoutes(await invoke<ModRoute[]>('list_modulations'))
    } catch { /* no engine in the browser preview */ }
  }, [])

  useEffect(() => { void refresh() }, [refresh])

  useEffect(() => {
    if (!trackId && tracks.length > 0) setTrackId(tracks[0].id)
  }, [tracks, trackId])

  const track = tracks.find(t => t.id === trackId)
  const slots = useMemo(() => track?.inserts ?? [], [track])

  useEffect(() => {
    if (slots.length === 0) { setSlotId(''); return }
    if (!slots.some(s => s.id === slotId)) setSlotId(slots[0].id)
  }, [slots, slotId])

  useEffect(() => {
    if (!trackId || !slotId) { setParams([]); setParamId(-1); return }
    let cancelled = false
    invoke<ParamInfo[]>('get_plugin_parameters', { trackId, slotId })
      .then(list => {
        if (cancelled) return
        setParams(list)
        setParamId(list[0]?.id ?? -1)
      })
      .catch(() => { if (!cancelled) { setParams([]); setParamId(-1) } })
    return () => { cancelled = true }
  }, [trackId, slotId])

  const add = useCallback(async () => {
    const param = params.find(p => p.id === paramId)
    if (!track || !slotId || !param) return
    const rate = RATES[rateIndex]
    try {
      await invoke('add_modulation', {
        name: `${param.name} ${rate.label}`,
        trackId: track.id,
        slotId,
        paramId: param.id,
        shape,
        syncNum: rate.num,
        syncDen: rate.den,
        hz: null,
        // The knob's own value is the middle of the swing, so switching
        // the route on does not jump the sound.
        center: Math.max(0, Math.min(1, param.value)),
        depth,
      })
      await refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not add that route', { detail: String(e) })
    }
  }, [track, slotId, paramId, params, shape, rateIndex, depth, refresh])

  const update = useCallback(async (id: string, patch: Record<string, unknown>) => {
    setRoutes(prev => prev.map(r => (r.id === id ? { ...r, ...patch } as ModRoute : r)))
    try {
      await invoke('set_modulation', { id, ...patch })
    } catch { /* the control already moved; the next refresh corrects it */ }
  }, [])

  const remove = useCallback(async (id: string) => {
    try {
      await invoke('delete_modulation', { id })
      await refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not remove that route', { detail: String(e) })
    }
  }, [refresh])

  const targetLabel = useCallback((route: ModRoute) => {
    const t = tracks.find(x => x.id === route.track_id)
    const slot = t?.inserts.find(i => i.id === route.target.PluginParam.slot_id)
    return `${t?.name ?? 'track'} · ${slot?.pluginName ?? 'plug-in'} · parameter ${route.target.PluginParam.param_id}`
  }, [tracks])

  return (
    <DialogFrame
      title="Modulation"
      subtitle="A shape that keeps running, wired to a knob"
      onClose={onClose}
      width={720}
    >
      <div style={{ overflowY: 'auto', flex: 1, padding: 10 }}>
        <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}>
          <thead>
            <tr style={{ color: hw.textSecondary, borderBottom: `1px solid ${hw.border}` }}>
              <th style={th()}>On</th>
              <th style={th()}>Moves</th>
              <th style={th()}>Shape</th>
              <th style={th()}>Rate</th>
              <th style={th()}>Centre</th>
              <th style={th()}>Depth</th>
              <th style={th()} />
            </tr>
          </thead>
          <tbody>
            {routes.length === 0 && (
              <tr><td style={td()} colSpan={7}>
                <span style={{ color: hw.textFaint }}>
                  Nothing wired yet. Pick a plug-in knob below and give it a shape.
                </span>
              </td></tr>
            )}
            {routes.map(route => (
              <tr key={route.id} style={{ borderBottom: `1px solid ${hw.border}` }}>
                <td style={td()}>
                  <input
                    type="checkbox"
                    checked={route.enabled}
                    onChange={() => { void update(route.id, { enabled: !route.enabled }) }}
                    style={{ accentColor: hw.accent }}
                  />
                </td>
                <td style={td()}>{targetLabel(route)}</td>
                <td style={td()}>{shapeLabel(route)}</td>
                <td style={td()}>{rateLabel(route)}</td>
                <td style={td()}>
                  <input
                    type="range" min={0} max={1} step={0.01}
                    value={route.center}
                    onChange={(e) => { void update(route.id, { center: Number(e.target.value) }) }}
                    style={{ width: 90, accentColor: hw.accent }}
                  />
                </td>
                <td style={td()}>
                  <input
                    type="range" min={-1} max={1} step={0.01}
                    value={route.depth}
                    onChange={(e) => { void update(route.id, { depth: Number(e.target.value) }) }}
                    style={{ width: 90, accentColor: hw.accent }}
                  />
                </td>
                <td style={{ ...td(), textAlign: 'right' }}>
                  <button onClick={() => { void remove(route.id) }} style={btn()}>Remove</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div style={{
        padding: '8px 12px', display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap',
        background: hw.bgElevated, borderTop: `1px solid ${hw.border}`,
      }}>
        <span style={{ fontSize: 12, color: hw.textSecondary }}>Wire:</span>
        <select value={trackId} onChange={e => setTrackId(e.target.value)} style={sel()}>
          {tracks.length === 0 && <option value="">(no tracks)</option>}
          {tracks.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}
        </select>
        <select value={slotId} onChange={e => setSlotId(e.target.value)} style={sel()}>
          {slots.length === 0 && <option value="">(no plug-ins on that track)</option>}
          {slots.map(s => <option key={s.id} value={s.id}>{s.pluginName}</option>)}
        </select>
        <select value={paramId} onChange={e => setParamId(Number(e.target.value))} style={sel()}>
          {params.length === 0 && <option value={-1}>(no parameters)</option>}
          {params.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
        <select value={shape} onChange={e => setShape(e.target.value)} style={sel()}>
          {SHAPES.map(s => <option key={s.value} value={s.value}>{s.label}</option>)}
        </select>
        <select value={rateIndex} onChange={e => setRateIndex(Number(e.target.value))} style={sel()}>
          {RATES.map((r, i) => <option key={r.label} value={i}>{r.label}</option>)}
        </select>
        <label style={{ fontSize: 12, color: hw.textSecondary, display: 'flex', alignItems: 'center', gap: 4 }}>
          Depth
          <input
            type="range" min={-1} max={1} step={0.05}
            value={depth}
            onChange={e => setDepth(Number(e.target.value))}
            style={{ width: 90, accentColor: hw.accent }}
          />
        </label>
        <button onClick={() => { void add() }} style={btn(true)}>Add</button>
        <span style={{ fontSize: 11, color: hw.textFaint }}>
          The knob's own value becomes the middle of the swing, so switching a route on does not jump the sound.
        </span>
      </div>
    </DialogFrame>
  )
}

