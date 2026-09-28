import { useCallback, useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useTrackStore } from '../stores/trackStore'
import { useNotificationStore } from '../stores/notificationStore'

/**
 * Macro knobs: one control that moves several parameters at once.
 *
 * "Open the filter" on a hard dance patch is the cutoff on two synths, a
 * send and a touch of drive. Drawn by hand that is four automation lanes
 * kept in step by memory. Here it is one knob, and each link says which
 * two values the parameter travels between, so one link can open while
 * another closes.
 *
 * The model is saved with the song (`project.macros`), so a patch opens
 * where it was left.
 */

type MacroTarget =
  | 'TrackVolume'
  | 'TrackPan'
  | { PluginParam: { slot_id: string; param_id: number } }

interface MacroLink {
  id: string
  track_id: string
  target: MacroTarget
  min: number
  max: number
}

interface MacroDef {
  id: string
  name: string
  value: number
  links: MacroLink[]
}

interface ParamInfo {
  id: number
  name: string
  defaultValue: number
  value: number
  min: number
  max: number
  unit: string
  automatable: boolean
}

export function MacroPanel({ onClose }: { onClose: () => void }) {
  const tracks = useTrackStore(s => s.tracks)
  const [macros, setMacros] = useState<MacroDef[]>([])
  const [selectedId, setSelectedId] = useState<string>('')

  const [linkTrackId, setLinkTrackId] = useState<string>('')
  const [linkWhat, setLinkWhat] = useState<string>('volume')
  const [params, setParams] = useState<ParamInfo[]>([])
  const [linkParamId, setLinkParamId] = useState<number>(-1)

  const refresh = useCallback(async () => {
    try {
      const list = await invoke<MacroDef[]>('list_macros')
      setMacros(list)
      setSelectedId(prev => (list.some(m => m.id === prev) ? prev : (list[0]?.id ?? '')))
    } catch { /* no engine in the browser preview */ }
  }, [])

  useEffect(() => { void refresh() }, [refresh])

  useEffect(() => {
    if (!linkTrackId && tracks.length > 0) setLinkTrackId(tracks[0].id)
  }, [tracks, linkTrackId])

  const selected = macros.find(m => m.id === selectedId)
  const linkTrack = tracks.find(t => t.id === linkTrackId)
  const slots = linkTrack?.inserts ?? []

  // Parameters are read from the chosen plug-in, so the range boxes start
  // at the parameter's own range instead of a guess.
  useEffect(() => {
    const slotId = linkWhat.startsWith('slot:') ? linkWhat.slice(5) : ''
    if (!slotId || !linkTrackId) { setParams([]); setLinkParamId(-1); return }
    let cancelled = false
    invoke<ParamInfo[]>('get_plugin_parameters', { trackId: linkTrackId, slotId })
      .then(list => {
        if (cancelled) return
        setParams(list)
        setLinkParamId(list[0]?.id ?? -1)
      })
      .catch(() => { if (!cancelled) { setParams([]); setLinkParamId(-1) } })
    return () => { cancelled = true }
  }, [linkWhat, linkTrackId])

  // Saved links only carry a parameter number. The names come from the
  // plug-in, so they are looked up once per slot and remembered, and a row
  // reads "Cutoff" instead of "parameter 4".
  const [paramNames, setParamNames] = useState<Record<string, string>>({})
  useEffect(() => {
    const wanted = new Set<string>()
    for (const m of macros) {
      for (const l of m.links) {
        if (typeof l.target === 'object') wanted.add(`${l.track_id}|${l.target.PluginParam.slot_id}`)
      }
    }
    for (const key of wanted) {
      const [trackId, slotId] = key.split('|')
      invoke<ParamInfo[]>('get_plugin_parameters', { trackId, slotId })
        .then(list => setParamNames(prev => {
          const next = { ...prev }
          for (const p of list) next[`${key}|${p.id}`] = p.name
          return next
        }))
        .catch(() => { /* the number stays, which is still a true answer */ })
    }
  }, [macros])

  const targetLabel = useCallback((link: MacroLink) => {
    const track = tracks.find(t => t.id === link.track_id)
    const trackName = track ? track.name : `track ${link.track_id.slice(0, 6)}`
    if (link.target === 'TrackVolume') return `${trackName} · volume`
    if (link.target === 'TrackPan') return `${trackName} · pan`
    const slotId = link.target.PluginParam.slot_id
    const slot = track?.inserts.find(i => i.id === slotId)
    const slotName = slot ? slot.pluginName : `slot ${slotId.slice(0, 6)}`
    const paramId = link.target.PluginParam.param_id
    const paramName = paramNames[`${link.track_id}|${slotId}|${paramId}`]
    return `${trackName} · ${slotName} · ${paramName ?? `parameter ${paramId}`}`
  }, [tracks, paramNames])

  const newMacro = useCallback(async () => {
    const name = window.prompt('Name this macro', 'Open')?.trim()
    if (!name) return
    try {
      const id = await invoke<string>('add_macro', { name })
      await refresh()
      setSelectedId(id)
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not add that macro', { detail: String(e) })
    }
  }, [refresh])

  const removeMacro = useCallback(async (id: string) => {
    try {
      await invoke('delete_macro', { id })
      await refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not remove that macro', { detail: String(e) })
    }
  }, [refresh])

  const renameMacro = useCallback(async (macro: MacroDef) => {
    const name = window.prompt('Rename this macro', macro.name)?.trim()
    if (!name || name === macro.name) return
    try {
      await invoke('rename_macro', { id: macro.id, name })
      await refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not rename that macro', { detail: String(e) })
    }
  }, [refresh])

  // The knob writes straight through: the value is what the links follow,
  // and the engine gets every step so the sweep is heard while it happens.
  const turn = useCallback(async (id: string, value: number) => {
    setMacros(prev => prev.map(m => (m.id === id ? { ...m, value } : m)))
    try {
      await invoke('set_macro_value', { id, value })
    } catch { /* the readout already moved; the next refresh corrects it */ }
  }, [])

  const addLink = useCallback(async () => {
    if (!selected || !linkTrackId) return
    const isParam = linkWhat.startsWith('slot:')
    const param = params.find(p => p.id === linkParamId)
    if (isParam && !param) return
    const args: Record<string, unknown> = {
      id: selected.id,
      trackId: linkTrackId,
      min: isParam ? param!.min : (linkWhat === 'pan' ? -1 : -60),
      max: isParam ? param!.max : (linkWhat === 'pan' ? 1 : 0),
    }
    if (isParam) {
      args.slotId = linkWhat.slice(5)
      args.paramId = linkParamId
    } else {
      args.trackTarget = linkWhat
    }
    try {
      await invoke('add_macro_link', args)
      await refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not link that', { detail: String(e) })
    }
  }, [selected, linkTrackId, linkWhat, linkParamId, params, refresh])

  const setRange = useCallback(async (linkId: string, min: number, max: number) => {
    if (!selected) return
    setMacros(prev => prev.map(m => (m.id === selected.id
      ? { ...m, links: m.links.map(l => (l.id === linkId ? { ...l, min, max } : l)) }
      : m)))
    try {
      await invoke('set_macro_link_range', { id: selected.id, linkId, min, max })
    } catch { /* ignore: the boxes already show what was typed */ }
  }, [selected])

  const removeLink = useCallback(async (linkId: string) => {
    if (!selected) return
    try {
      await invoke('remove_macro_link', { id: selected.id, linkId })
      await refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not unlink that', { detail: String(e) })
    }
  }, [selected, refresh])

  const linkRows = useMemo(() => selected?.links ?? [], [selected])

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 9800,
        background: 'rgba(0,0,0,0.45)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div style={{
        width: 720, maxWidth: '95vw', maxHeight: '82vh',
        background: hw.bg, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg,
        overflow: 'hidden', display: 'flex', flexDirection: 'column',
      }}>
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 12,
          background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Macro knobs</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>
            one knob, several parameters · saved with the song
          </div>
          <div style={{ flex: 1 }} />
          <button onClick={() => { void newMacro() }} style={btn()}>New macro</button>
          <button onClick={onClose} style={btn()}>Close</button>
        </div>

        <div style={{ display: 'flex', minHeight: 0, flex: 1 }}>
          <div style={{
            width: 180, borderRight: `1px solid ${hw.border}`,
            overflowY: 'auto', background: hw.bgElevated,
          }}>
            {macros.length === 0 && (
              <div style={{ padding: 12, fontSize: 10, color: hw.textFaint }}>
                No macros yet. Make one, then link the knobs it should move.
              </div>
            )}
            {macros.map(m => (
              <div
                key={m.id}
                onClick={() => setSelectedId(m.id)}
                onDoubleClick={() => { void renameMacro(m) }}
                style={{
                  padding: '7px 10px', fontSize: 11, cursor: 'pointer',
                  background: m.id === selectedId ? hw.bg : 'transparent',
                  borderLeft: `2px solid ${m.id === selectedId ? hw.accent : 'transparent'}`,
                  display: 'flex', alignItems: 'center', gap: 6,
                }}
              >
                <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis' }}>{m.name}</span>
                <span style={{ fontSize: 9, color: hw.textFaint, fontVariantNumeric: 'tabular-nums' }}>
                  {Math.round(m.value * 100)}
                </span>
              </div>
            ))}
          </div>

          <div style={{ flex: 1, minWidth: 0, overflowY: 'auto', padding: 12 }}>
            {!selected && (
              <div style={{ fontSize: 10, color: hw.textFaint }}>
                Pick a macro on the left, or make one.
              </div>
            )}
            {selected && (
              <>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 12 }}>
                  <div style={{ fontSize: 12, fontWeight: 600 }}>{selected.name}</div>
                  <input
                    type="range" min={0} max={1} step={0.001}
                    value={selected.value}
                    onChange={(e) => { void turn(selected.id, Number(e.target.value)) }}
                    style={{ flex: 1, accentColor: hw.accent }}
                  />
                  <div style={{ width: 34, textAlign: 'right', fontSize: 10, fontVariantNumeric: 'tabular-nums' }}>
                    {Math.round(selected.value * 100)}
                  </div>
                  <button onClick={() => { void renameMacro(selected) }} style={btn()}>Rename</button>
                  <button onClick={() => { void removeMacro(selected.id) }} style={btn()}>Delete</button>
                </div>

                <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 10 }}>
                  <thead>
                    <tr style={{ color: hw.textSecondary, borderBottom: `1px solid ${hw.border}` }}>
                      <th style={th()}>Moves</th>
                      <th style={th()}>At 0</th>
                      <th style={th()}>At 100</th>
                      <th style={th()} />
                    </tr>
                  </thead>
                  <tbody>
                    {linkRows.length === 0 && (
                      <tr><td style={td()} colSpan={4}>
                        <span style={{ color: hw.textFaint }}>
                          Nothing linked yet. Pick a track and a parameter below.
                        </span>
                      </td></tr>
                    )}
                    {linkRows.map(link => (
                      <tr key={link.id} style={{ borderBottom: `1px solid ${hw.border}` }}>
                        <td style={td()}>{targetLabel(link)}</td>
                        <td style={td()}>
                          <input
                            type="number" value={link.min} step="any"
                            onChange={(e) => { void setRange(link.id, Number(e.target.value), link.max) }}
                            style={num()}
                          />
                        </td>
                        <td style={td()}>
                          <input
                            type="number" value={link.max} step="any"
                            onChange={(e) => { void setRange(link.id, link.min, Number(e.target.value)) }}
                            style={num()}
                          />
                        </td>
                        <td style={td()}>
                          <button onClick={() => { void removeLink(link.id) }} style={btn()}>Unlink</button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>

                <div style={{
                  marginTop: 12, paddingTop: 10, borderTop: `1px solid ${hw.border}`,
                  display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap',
                }}>
                  <span style={{ fontSize: 10, color: hw.textSecondary }}>Link:</span>
                  <select value={linkTrackId} onChange={(e) => setLinkTrackId(e.target.value)} style={sel()}>
                    {tracks.length === 0 && <option value="">(no tracks)</option>}
                    {tracks.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}
                  </select>
                  <select value={linkWhat} onChange={(e) => setLinkWhat(e.target.value)} style={sel()}>
                    <option value="volume">Volume</option>
                    <option value="pan">Pan</option>
                    {slots.map(s => (
                      <option key={s.id} value={`slot:${s.id}`}>{s.pluginName}</option>
                    ))}
                  </select>
                  {linkWhat.startsWith('slot:') && (
                    <select
                      value={linkParamId}
                      onChange={(e) => setLinkParamId(Number(e.target.value))}
                      style={sel()}
                    >
                      {params.length === 0 && <option value={-1}>(no parameters)</option>}
                      {params.map(p => (
                        <option key={p.id} value={p.id}>{p.name}</option>
                      ))}
                    </select>
                  )}
                  <button onClick={() => { void addLink() }} style={btn(true)}>Add</button>
                  <span style={{ fontSize: 9, color: hw.textFaint }}>
                    Put the larger number in the "At 0" box to make a link run backwards.
                  </span>
                </div>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

function btn(active: boolean = false) {
  return {
    padding: '3px 10px', fontSize: 10, background: 'transparent',
    border: `1px solid ${active ? hw.accent : hw.border}`, borderRadius: hw.radius.sm,
    color: active ? hw.accent : hw.textSecondary, cursor: 'pointer',
  } as const
}

function sel() {
  return {
    fontSize: 10, padding: '2px 6px', background: hw.bg,
    border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
    color: hw.textPrimary, maxWidth: 200,
  } as const
}

function num() {
  return {
    fontSize: 10, padding: '2px 4px', width: 78, background: hw.bg,
    border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
    color: hw.textPrimary, fontVariantNumeric: 'tabular-nums' as const,
  } as const
}

function th() {
  return { textAlign: 'left' as const, padding: '6px 8px', fontWeight: 600 }
}

function td() {
  return { padding: '6px 8px', fontVariantNumeric: 'tabular-nums' as const }
}
