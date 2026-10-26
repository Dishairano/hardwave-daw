import { useCallback, useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useTrackStore } from '../stores/trackStore'
import { useNotificationStore } from '../stores/notificationStore'
import { useSendStore } from '../stores/sendStore'
import { DialogFrame } from './ui/DialogFrame'
import { btn, sel, th, td } from './ui/dialogStyles'

/**
 * The modulation matrix: a source that keeps running, wired to a knob.
 *
 * Automation draws a shape once and plays it back. A filter that
 * breathes for the whole song needed a lane full of points; here it is
 * one route, and the shape never stops. The source can also be another
 * track's level (a kick ducking a bass) or a controller on the keyboard,
 * and the route can move a track's volume, pan or a send as well as a
 * plug-in's knob.
 */

type LfoShape = 'Sine' | 'Triangle' | 'Square' | 'SawtoothUp' | 'SawtoothDown' | 'RandomSampleAndHold'

type ModSource =
  | { Lfo: { shape: LfoShape; rate: { TempoSync?: { num: number; den: number }; Hz?: number }; phase_offset: number } }
  | { Envelope: { track_id: string; release_ms: number } }
  | { MidiCc: { cc: number } }

type ModTarget =
  | { PluginParam: { slot_id: string; param_id: number } }
  | 'TrackVolume'
  | 'TrackPan'
  | { SendLevel: { send_index: number } }

interface ModRoute {
  id: string
  name: string
  enabled: boolean
  source: ModSource
  track_id: string
  target: ModTarget
  center: number
  depth: number
}

type SourceKind = 'lfo' | 'envelope' | 'cc'
// 'plugin', 'volume', 'pan', or 'send:<index>'
type TargetKey = string

const RELEASES = [30, 60, 120, 250, 500, 1000]

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
  if ('Envelope' in route.source) return `falls in ${route.source.Envelope.release_ms} ms`
  if (!('Lfo' in route.source)) return ''
  const rate = route.source.Lfo.rate
  if (rate.Hz !== undefined) return `${rate.Hz} Hz`
  const sync = rate.TempoSync
  if (!sync) return 'free'
  const match = RATES.find(r => r.num === sync.num && r.den === sync.den)
  return match ? match.label : `${sync.num}/${sync.den}`
}

function shapeLabel(route: ModRoute, trackName: (id: string) => string): string {
  const source = route.source
  if ('Envelope' in source) return `Level of ${trackName(source.Envelope.track_id)}`
  if ('MidiCc' in source) return source.MidiCc.cc === 1 ? 'Mod wheel (CC 1)' : `Controller ${source.MidiCc.cc}`
  return SHAPES.find(s => s.stored === source.Lfo.shape)?.label ?? source.Lfo.shape
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
  const [sourceKind, setSourceKind] = useState<SourceKind>('lfo')
  const [followId, setFollowId] = useState('')
  const [releaseMs, setReleaseMs] = useState(120)
  const [cc, setCc] = useState(1)
  const [targetKey, setTargetKey] = useState<TargetKey>('plugin')
  const sendsByTrack = useSendStore(s => s.byTrack)
  const fetchSends = useSendStore(s => s.fetchAll)

  const refresh = useCallback(async () => {
    try {
      setRoutes(await invoke<ModRoute[]>('list_modulations'))
    } catch { /* no engine in the browser preview */ }
  }, [])

  useEffect(() => { void refresh() }, [refresh])
  useEffect(() => { void fetchSends().catch(() => {}) }, [fetchSends])

  useEffect(() => {
    if (!trackId && tracks.length > 0) setTrackId(tracks[0].id)
  }, [tracks, trackId])

  const track = tracks.find(t => t.id === trackId)
  const slots = useMemo(() => track?.inserts ?? [], [track])
  const sends = sendsByTrack[trackId] ?? []
  const trackName = useCallback((id: string) => tracks.find(t => t.id === id)?.name ?? 'a track', [tracks])

  // The followed track is another one: a track's volume can't follow its
  // own level, and following yourself is rarely what anyone means.
  useEffect(() => {
    if (followId && followId !== trackId && tracks.some(t => t.id === followId)) return
    setFollowId(tracks.find(t => t.id !== trackId)?.id ?? '')
  }, [tracks, trackId, followId])

  useEffect(() => {
    if (targetKey.startsWith('send:') && !sends.some(s => `send:${s.index}` === targetKey)) setTargetKey('plugin')
  }, [sends, targetKey])

  // A level or a controller sits at rest at the bottom of its swing, so
  // the useful way round for a level is down: a duck.
  useEffect(() => {
    setDepth(sourceKind === 'lfo' ? 0.3 : (targetKey === 'pan' ? 0.5 : -0.3))
  }, [sourceKind, targetKey])

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
    if (!track) return
    const param = params.find(p => p.id === paramId)
    if (targetKey === 'plugin' && (!slotId || !param)) return
    const rate = RATES[rateIndex]
    // Where the route sits with the source in the middle. A knob keeps
    // its own value, so switching the route on does not jump the sound.
    // Volume and a send are multiplied by the route and can't go past
    // what the fader says, so the swing is placed to top out at 1 (with a
    // level or a controller: to sit at 1 at rest). Pan sits in the middle.
    const restTop = sourceKind === 'lfo' ? 1 - Math.abs(depth) : 1 + Math.min(depth, 0)
    const center = targetKey === 'plugin'
      ? Math.max(0, Math.min(1, param?.value ?? 0.5))
      : targetKey === 'pan' ? 0.5 : restTop
    const sourceLabel = sourceKind === 'envelope'
      ? `from ${trackName(followId)}`
      : sourceKind === 'cc' ? `CC ${cc}` : rate.label
    const targetLabel = targetKey === 'plugin' ? param?.name ?? 'Knob'
      : targetKey === 'volume' ? 'Volume' : targetKey === 'pan' ? 'Pan' : 'Send'
    try {
      await invoke('add_modulation', {
        name: `${targetLabel} ${sourceLabel}`,
        trackId: track.id,
        slotId: targetKey === 'plugin' ? slotId : null,
        paramId: targetKey === 'plugin' ? param?.id ?? null : null,
        shape,
        syncNum: rate.num,
        syncDen: rate.den,
        hz: null,
        center: Math.max(0, Math.min(1, center)),
        depth,
        source: sourceKind,
        sourceTrackId: sourceKind === 'envelope' ? followId : null,
        releaseMs: sourceKind === 'envelope' ? releaseMs : null,
        cc: sourceKind === 'cc' ? cc : null,
        target: targetKey.startsWith('send:') ? 'send' : targetKey,
        sendIndex: targetKey.startsWith('send:') ? Number(targetKey.slice(5)) : null,
      })
      await refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not add that route', { detail: String(e) })
    }
  }, [track, slotId, paramId, params, shape, rateIndex, depth, refresh, sourceKind, followId, releaseMs, cc, targetKey, trackName])

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
    const target = route.target
    if (target === 'TrackVolume') return `${t?.name ?? 'track'} · volume`
    if (target === 'TrackPan') return `${t?.name ?? 'track'} · pan`
    if ('SendLevel' in target) return `${t?.name ?? 'track'} · send ${target.SendLevel.send_index + 1}`
    const slot = t?.inserts.find(i => i.id === target.PluginParam.slot_id)
    return `${t?.name ?? 'track'} · ${slot?.pluginName ?? 'plug-in'} · parameter ${target.PluginParam.param_id}`
  }, [tracks])

  return (
    <DialogFrame
      title="Modulation"
      subtitle="A shape, a track's level or a controller, wired to a knob, a fader, a pan or a send"
      onClose={onClose}
      width={720}
    >
      <div style={{ overflowY: 'auto', flex: 1, padding: 10 }}>
        <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}>
          <thead>
            <tr style={{ color: hw.textSecondary, borderBottom: `1px solid ${hw.border}` }}>
              <th style={th()}>On</th>
              <th style={th()}>Moves</th>
              <th style={th()}>From</th>
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
                  Nothing wired yet. Pick what moves below and where it moves from.
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
                <td style={td()}>{shapeLabel(route, trackName)}</td>
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
        <span style={{ fontSize: 12, color: hw.textSecondary }}>Moves:</span>
        <select value={trackId} onChange={e => setTrackId(e.target.value)} style={sel()}>
          {tracks.length === 0 && <option value="">(no tracks)</option>}
          {tracks.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}
        </select>
        <select value={targetKey} onChange={e => setTargetKey(e.target.value)} style={sel()}>
          <option value="plugin">A plug-in knob</option>
          <option value="volume">Volume</option>
          <option value="pan">Pan</option>
          {sends.map(s => (
            <option key={s.index} value={`send:${s.index}`}>Send {s.index + 1} to {trackName(s.target)}</option>
          ))}
        </select>
        {targetKey === 'plugin' && (
          <>
            <select value={slotId} onChange={e => setSlotId(e.target.value)} style={sel()}>
              {slots.length === 0 && <option value="">(no plug-ins on that track)</option>}
              {slots.map(s => <option key={s.id} value={s.id}>{s.pluginName}</option>)}
            </select>
            <select value={paramId} onChange={e => setParamId(Number(e.target.value))} style={sel()}>
              {params.length === 0 && <option value={-1}>(no parameters)</option>}
              {params.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
            </select>
          </>
        )}
        <span style={{ fontSize: 12, color: hw.textSecondary, marginLeft: 6 }}>From:</span>
        <select value={sourceKind} onChange={e => setSourceKind(e.target.value as SourceKind)} style={sel()}>
          <option value="lfo">A shape (LFO)</option>
          <option value="envelope">Another track's level</option>
          <option value="cc">A MIDI controller</option>
        </select>
        {sourceKind === 'lfo' && (
          <>
            <select value={shape} onChange={e => setShape(e.target.value)} style={sel()}>
              {SHAPES.map(s => <option key={s.value} value={s.value}>{s.label}</option>)}
            </select>
            <select value={rateIndex} onChange={e => setRateIndex(Number(e.target.value))} style={sel()}>
              {RATES.map((r, i) => <option key={r.label} value={i}>{r.label}</option>)}
            </select>
          </>
        )}
        {sourceKind === 'envelope' && (
          <>
            <select value={followId} onChange={e => setFollowId(e.target.value)} style={sel()}>
              {tracks.filter(t => t.id !== trackId).map(t => <option key={t.id} value={t.id}>{t.name}</option>)}
            </select>
            <select value={releaseMs} onChange={e => setReleaseMs(Number(e.target.value))} style={sel()} title="How fast it lets go after the track gets quieter">
              {RELEASES.map(ms => <option key={ms} value={ms}>falls in {ms} ms</option>)}
            </select>
          </>
        )}
        {sourceKind === 'cc' && (
          <label style={{ fontSize: 12, color: hw.textSecondary, display: 'flex', alignItems: 'center', gap: 4 }}>
            CC
            <input
              type="number" min={0} max={127}
              value={cc}
              onChange={e => setCc(Math.max(0, Math.min(127, Math.round(Number(e.target.value) || 0))))}
              style={{ ...sel(), width: 56 }}
              title="1 is the mod wheel"
            />
          </label>
        )}
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
          {targetKey === 'plugin'
            ? "The knob's own value becomes the middle of the swing, so switching a route on does not jump the sound."
            : targetKey === 'pan'
              ? 'Moves the pan from where the pan knob has it.'
              : 'Turns the level down from where the fader has it, never past it. A negative depth with a level ducks: louder there, quieter here.'}
        </span>
      </div>
    </DialogFrame>
  )
}

