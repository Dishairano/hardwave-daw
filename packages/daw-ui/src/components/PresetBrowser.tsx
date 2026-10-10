import { useCallback, useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useTrackStore } from '../stores/trackStore'
import { usePluginStore } from '../stores/pluginStore'
import { useNotificationStore } from '../stores/notificationStore'
import { DialogFrame } from './ui/DialogFrame'

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
  /** The bank's plug-in is not in the scanned list on this computer. */
  notInstalled?: boolean
  preset: PresetInfo
  /// A preset that ships inside the plug-in rather than one you saved.
  factory?: boolean
  factoryIndex?: number
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

  // The presets a plug-in ships with, read from the plug-in itself the
  // first time its slot is picked. Kept per plug-in so switching slots
  // does not ask again.
  const [factory, setFactory] = useState<Record<string, string[]>>({})
  useEffect(() => {
    const slot = slots.find(s => s.id === targetSlotId)
    if (!slot || factory[slot.pluginId] !== undefined) return
    invoke<string[]>('list_factory_presets', { pluginId: slot.pluginId })
      .then(names => setFactory(prev => ({ ...prev, [slot.pluginId]: names })))
      .catch(() => setFactory(prev => ({ ...prev, [slot.pluginId]: [] })))
  }, [slots, targetSlotId, factory])

  const rows = useMemo<Row[]>(() => {
    const needle = query.trim().toLowerCase()
    const all: Row[] = []
    for (const bank of banks) {
      const descriptor = plugins.find(p => p.id === bank.pluginId)
      const pluginName = descriptor ? descriptor.name : readablePluginName(bank.pluginId)
      for (const preset of bank.presets) {
        if (needle
          && !preset.name.toLowerCase().includes(needle)
          && !pluginName.toLowerCase().includes(needle)) continue
        all.push({ pluginId: bank.pluginId, pluginName, preset, notInstalled: !descriptor })
      }
    }
    // The chosen slot's own presets, listed beside the saved ones.
    const slot = slots.find(s => s.id === targetSlotId)
    const own = slot ? (factory[slot.pluginId] ?? []) : []
    for (const [index, name] of own.entries()) {
      if (needle
        && !name.toLowerCase().includes(needle)
        && !(slot?.pluginName ?? '').toLowerCase().includes(needle)) continue
      all.push({
        pluginId: slot!.pluginId,
        pluginName: slot!.pluginName,
        preset: { id: `factory-${index}`, name, created_at: 0 },
        factory: true,
        factoryIndex: index,
      })
    }
    return all
  }, [banks, plugins, query, slots, targetSlotId, factory])

  // A preset holds one plug-in's state, so it only means something in a
  // slot running that plug-in. Loading it anywhere else would quietly do
  // nothing, which is worse than saying so.
  const targetSlot = slots.find(s => s.id === targetSlotId)
  const canLoad = useCallback((row: Row) => !!targetSlot && targetSlot.pluginId === row.pluginId, [targetSlot])

  const load = useCallback(async (row: Row) => {
    if (!targetSlot || !targetTrackId) return
    try {
      if (row.factory) {
        await invoke('load_factory_preset', {
          trackId: targetTrackId,
          slotId: targetSlot.id,
          index: row.factoryIndex ?? 0,
        })
        useNotificationStore.getState().push('info', `Loaded "${row.preset.name}"`, {
          detail: `${row.pluginName}'s own preset.`,
        })
        return
      }
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
    <DialogFrame
      title="Presets"
      width={860}
      subtitle="Everything you have saved, across every plug-in"
      onClose={onClose}
      headerActions={<button type="button" className="hw-dbtn" onClick={() => { void refresh() }}>Refresh</button>}
      footer={<span className="hw-dialog-note">A VST3's own presets are listed once you pick a slot running it. A CLAP's are not: those come through a factory the host does not read yet.</span>}
    >
      <div className="hw-dialog-bar">
        <input
          className="hw-input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search presets"
          style={{ flex: 1, minWidth: 160 }}
        />
        <span className="hw-dialog-lbl">Load into</span>
        <select className="hw-select" value={targetTrackId} onChange={(e) => setTargetTrackId(e.target.value)}>
          {tracks.length === 0 && <option value="">(no tracks)</option>}
          {tracks.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}
        </select>
        <select className="hw-select" value={targetSlotId} onChange={(e) => setTargetSlotId(e.target.value)}>
          {slots.length === 0 && <option value="">(no plug-ins on that track)</option>}
          {slots.map(s => <option key={s.id} value={s.id}>{s.pluginName}</option>)}
        </select>
      </div>

      <div style={{ overflowY: 'auto', flex: 1 }}>
        <table className="hw-table">
          <thead>
            <tr>
              <th>Preset</th>
              <th>Plug-in</th>
              <th>Saved</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr><td colSpan={4} className="hw-muted">
                {banks.length === 0
                  ? 'Nothing saved yet. Save a preset from a plug-in slot and it appears here.'
                  : 'Nothing matches that search.'}
              </td></tr>
            )}
            {rows.map(row => (
              <tr key={`${row.pluginId}:${row.preset.id}`}>
                <td><b>{row.preset.name}</b></td>
                <td className="hw-muted">
                  {row.pluginName}
                  {row.notInstalled && <span className="hw-tag">not installed</span>}
                </td>
                <td className="hw-faint">
                  {row.factory
                    ? 'From the plug-in'
                    : row.preset.created_at
                      ? new Date(row.preset.created_at * 1000).toLocaleDateString()
                      : ''}
                </td>
                <td style={{ textAlign: 'right', whiteSpace: 'nowrap' }}>
                  <button
                    type="button"
                    className={`hw-dbtn${canLoad(row) ? ' primary' : ''}`}
                    onClick={() => { void load(row) }}
                    disabled={!canLoad(row)}
                    title={canLoad(row)
                      ? 'Load into the chosen slot'
                      : 'Pick a slot running this plug-in first'}
                  >Load</button>
                  {/* A preset inside the plug-in is the plug-in's,
                      not ours: it cannot be renamed or deleted. */}
                  {!row.factory && (<>
                    <button type="button" className="hw-dbtn" onClick={() => { void rename(row) }}>Rename</button>
                    <button type="button" className="hw-dbtn danger" onClick={() => { void remove(row) }}>Delete</button>
                  </>)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </DialogFrame>
  )
}

// Our own plug-ins, spelled the way we write them.
const HARDWAVE_NAMES: Record<string, string> = {
  wettboi: 'WettBoi', wideboi: 'WideBoi', loudlab: 'LoudLab', pumpcontrol: 'PumpControl',
  kickforge: 'KickForge', analyser: 'Analyser', hardq: 'Hard-Q',
}

/** A name for a plug-in we only know by its id (it was shown as the raw id). */
function readablePluginName(id: string): string {
  const last = id.split(/[./:]/).filter(Boolean).pop() ?? id
  const known = HARDWAVE_NAMES[last.toLowerCase().replace(/[-_ ]/g, '')]
  if (known) return known
  return last.split(/[-_]/).filter(Boolean).map(w => w[0].toUpperCase() + w.slice(1)).join(' ')
}

