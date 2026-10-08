import { useEffect, useState } from 'react'
import { usePluginPresetStore } from '../../stores/pluginPresetStore'
import { useNotificationStore } from '../../stores/notificationStore'
import { PluginParamSheet } from './PluginParamSheet'

export interface PluginControlsProps {
  trackId: string
  slotId: string
  pluginId: string
  pluginName: string
  /** Shown as a × in the header when the controls sit in a panel. */
  onClose?: () => void
}

/**
 * A built-in plug-in's window: its name, a preset dropdown and its
 * parameters. Used as a floating panel beside a docked mixer and as a window
 * of its own beside a detached one.
 */
export function PluginControls({ trackId, slotId, pluginId, pluginName, onClose }: PluginControlsProps) {
  const presets = usePluginPresetStore((s) => s.byPlugin[pluginId] ?? null)
  // Bumped after a preset loads, so the sliders read the new values.
  const [generation, setGeneration] = useState(0)

  useEffect(() => {
    usePluginPresetStore.getState().refresh(pluginId).catch(() => { /* list stays empty */ })
  }, [pluginId])

  const report = (what: string) => (e: unknown) =>
    useNotificationStore.getState().push('error', `${what}: ${String(e)}`)

  const onPreset = async (value: string) => {
    if (!value) return
    if (value === '__save') {
      const name = window.prompt('Preset name:')?.trim()
      if (!name) return
      await usePluginPresetStore.getState().save(trackId, slotId, pluginId, name).catch(report('Could not save the preset'))
      return
    }
    await usePluginPresetStore.getState().load(trackId, slotId, pluginId, value).catch(report('Could not load the preset'))
    setGeneration((g) => g + 1)
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', minHeight: 0, height: '100%' }}>
      <div style={{
        display: 'flex', alignItems: 'center', gap: 8, padding: '8px 10px',
        borderBottom: '1px solid rgba(255,255,255,0.08)',
      }}>
        <span style={{
          flex: '0 1 auto', fontSize: 11, fontWeight: 600, color: '#fff',
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>
          {pluginName}
        </span>
        <select
          value=""
          onChange={(e) => void onPreset(e.target.value)}
          title="Presets"
          style={{
            flex: 1, minWidth: 0, background: '#0a0a0d', color: '#e5e5ea',
            border: '1px solid rgba(255,255,255,0.12)', fontSize: 11, padding: '3px 6px',
          }}
        >
          <option value="">{presets == null ? 'Presets…' : presets.length ? 'Presets' : 'No presets yet'}</option>
          {(presets ?? []).map((p) => (
            <option key={p.id} value={p.id}>{p.name}</option>
          ))}
          <option value="__save">+ Save current as preset…</option>
        </select>
        {onClose && (
          <button
            onClick={onClose}
            aria-label="Close"
            title="Close (Esc)"
            style={{ background: 'transparent', border: 0, color: '#9a9aa6', cursor: 'pointer', fontSize: 14, lineHeight: 1 }}
          >
            ×
          </button>
        )}
      </div>
      <div style={{ padding: 10, overflowY: 'auto', minHeight: 0 }}>
        <PluginParamSheet key={generation} trackId={trackId} slotId={slotId} fill />
      </div>
    </div>
  )
}
