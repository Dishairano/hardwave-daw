import { useCallback, useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useTrackStore } from '../stores/trackStore'
import { usePluginStore } from '../stores/pluginStore'
import { useNotificationStore } from '../stores/notificationStore'

/**
 * Every saved preset in one place.
 *
 * A preset used to be reachable only from the slot that made it, so a
 * patch saved while working on Insert 3 was invisible on Insert 11 and
 * there was no way to see what had been saved at all. This lists every
 * plug-in's presets, searches across them, and loads one into whichever
 * slot is picked.
 *
 * Factory presets that live inside a plug-in are not here: reading them
 * needs the program list a VST3 exposes, which the host crate does not
 * surface yet.
 */

interface PresetInfo {
  id: string
  name: string
  created_at: number
}

interface PresetBank {
  pluginId: string
  presets: PresetInfo[]
}

interface Row {
  pluginId: string
  pluginName: string
  preset: PresetInfo
}

export function PresetBrowser({ onClose }: { onClose: () => void }) {
  const tracks = useTrackStore(s => s.tracks)
  const plugins = usePluginStore(s => s.plugins)
  const [banks, setBanks] = useState<PresetBank[]>([])
  const [query, setQuery] = useState('')
  const [targetTrackId, setTargetTrackId] = useState('')
  const [targetSlotId, setTargetSlotId] = useState('')

  const refresh = useCallback(async () => {
    try {
      setBanks(await invoke<PresetBank[]>('list_all_presets'))
    } catch { /* no engine in the browser preview */ }
  }, [])

  useEffect(() => { void refresh() }, [refresh])

  useEffect(() => {
    if (!targetTrackId && tracks.length > 0) setTargetTrackId(tracks[0].id)
  }, [tracks, targetTrackId])

  const targetTrack = tracks.find(t => t.id === targetTrackId)
  const slots = useMemo(() => targetTrack?.inserts ?? [], [targetTrack])

  useEffect(() => {
    if (slots.length === 0) { setTargetSlotId(''); return }
    if (!slots.some(s => s.id === targetSlotId)) setTargetSlotId(slots[0].id)
  }, [slots, targetSlotId])

  const rows = useMemo<Row[]>(() => {
    const needle = query.trim().toLowerCase()
    const all: Row[] = []
    for (const bank of banks) {
      const descriptor = plugins.find(p => p.id === bank.pluginId)
      const pluginName = descriptor ? descriptor.name : bank.pluginId
      for (const preset of bank.presets) {
        if (needle
          && !preset.name.toLowerCase().includes(needle)
          && !pluginName.toLowerCase().includes(needle)) continue
        all.push({ pluginId: bank.pluginId, pluginName, preset })
      }
    }
    return all
  }, [banks, plugins, query])

  // A preset holds one plug-in's state, so it only means something in a
  // slot running that plug-in. Loading it anywhere else would quietly do
  // nothing, which is worse than saying so.
  const targetSlot = slots.find(s => s.id === targetSlotId)
  const canLoad = useCallback((row: Row) => !!targetSlot && targetSlot.pluginId === row.pluginId, [targetSlot])

  const load = useCallback(async (row: Row) => {
    if (!targetSlot || !targetTrackId) return
    try {
      await invoke('load_plugin_preset', {
        trackId: targetTrackId,
        slotId: targetSlot.id,
        pluginId: row.pluginId,
        presetId: row.preset.id,
      })
      useNotificationStore.getState().push('info', `Loaded "${row.preset.name}"`, {
        detail: `${row.pluginName} on ${targetTrack?.name ?? 'the track'}.`,
      })
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not load that preset', { detail: String(e) })
    }
  }, [targetSlot, targetTrackId, targetTrack])

  const rename = useCallback(async (row: Row) => {
    const newName = window.prompt('Rename this preset', row.preset.name)?.trim()
    if (!newName || newName === row.preset.name) return
    try {
      await invoke('rename_plugin_preset', {
        pluginId: row.pluginId, presetId: row.preset.id, newName,
      })
      await refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not rename that preset', { detail: String(e) })
    }
  }, [refresh])

  const remove = useCallback(async (row: Row) => {
    try {
      await invoke('delete_plugin_preset', { pluginId: row.pluginId, presetId: row.preset.id })
      await refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not delete that preset', { detail: String(e) })
    }
  }, [refresh])

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
        width: 700, maxWidth: '95vw', maxHeight: '82vh',
        background: hw.bg, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg,
        overflow: 'hidden', display: 'flex', flexDirection: 'column',
      }}>
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 12,
          background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Presets</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>
            everything you have saved, across every plug-in
          </div>
          <div style={{ flex: 1 }} />
          <button onClick={() => { void refresh() }} style={btn()}>Refresh</button>
          <button onClick={onClose} style={btn()}>Close</button>
        </div>

        <div style={{
          padding: 10, display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap',
          background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search presets"
            style={{
              fontSize: 10, padding: '3px 8px', flex: 1, minWidth: 160,
              background: hw.bg, border: `1px solid ${hw.border}`,
              borderRadius: hw.radius.sm, color: hw.textPrimary,
            }}
          />
          <span style={{ fontSize: 10, color: hw.textSecondary }}>Load into:</span>
          <select value={targetTrackId} onChange={(e) => setTargetTrackId(e.target.value)} style={sel()}>
            {tracks.length === 0 && <option value="">(no tracks)</option>}
            {tracks.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}
          </select>
          <select value={targetSlotId} onChange={(e) => setTargetSlotId(e.target.value)} style={sel()}>
            {slots.length === 0 && <option value="">(no plug-ins on that track)</option>}
            {slots.map(s => <option key={s.id} value={s.id}>{s.pluginName}</option>)}
          </select>
        </div>

        <div style={{ overflowY: 'auto', flex: 1 }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 10 }}>
            <thead>
              <tr style={{
                color: hw.textSecondary, borderBottom: `1px solid ${hw.border}`,
                position: 'sticky', top: 0, background: hw.bg,
              }}>
                <th style={th()}>Preset</th>
                <th style={th()}>Plug-in</th>
                <th style={th()}>Saved</th>
                <th style={th()} />
              </tr>
            </thead>
            <tbody>
              {rows.length === 0 && (
                <tr><td style={td()} colSpan={4}>
                  <span style={{ color: hw.textFaint }}>
                    {banks.length === 0
                      ? 'Nothing saved yet. Save a preset from a plug-in slot and it appears here.'
                      : 'Nothing matches that search.'}
                  </span>
                </td></tr>
              )}
              {rows.map(row => (
                <tr key={`${row.pluginId}:${row.preset.id}`} style={{ borderBottom: `1px solid ${hw.border}` }}>
                  <td style={td()}>{row.preset.name}</td>
                  <td style={{ ...td(), color: hw.textSecondary }}>{row.pluginName}</td>
                  <td style={{ ...td(), color: hw.textFaint }}>
                    {row.preset.created_at
                      ? new Date(row.preset.created_at * 1000).toLocaleDateString()
                      : ''}
                  </td>
                  <td style={{ ...td(), textAlign: 'right', whiteSpace: 'nowrap' }}>
                    <button
                      onClick={() => { void load(row) }}
                      disabled={!canLoad(row)}
                      title={canLoad(row)
                        ? 'Load into the chosen slot'
                        : 'Pick a slot running this plug-in first'}
                      style={{ ...btn(canLoad(row)), opacity: canLoad(row) ? 1 : 0.4 }}
                    >Load</button>
                    {' '}
                    <button onClick={() => { void rename(row) }} style={btn()}>Rename</button>
                    {' '}
                    <button onClick={() => { void remove(row) }} style={btn()}>Delete</button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <div style={{
          padding: '6px 12px', fontSize: 9, color: hw.textFaint,
          borderTop: `1px solid ${hw.border}`, background: hw.bgElevated,
        }}>
          Presets built into a plug-in are not listed. Reading those needs the
          program list a VST3 publishes, which the host does not read yet.
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

function th() {
  return { textAlign: 'left' as const, padding: '6px 8px', fontWeight: 600 }
}

function td() {
  return { padding: '6px 8px', fontVariantNumeric: 'tabular-nums' as const }
}
