import { useState, type CSSProperties } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useMissingPluginsStore, type MissingPluginInfo } from '../stores/missingPluginsStore'
import { useNotificationStore } from '../stores/notificationStore'

/**
 * Persistent banner for projects that reference uninstalled plugins.
 * Replaces the one-shot dialog (deep-research P0-5 remainder): the
 * dialog was easy to dismiss and forget, after which the mix silently
 * played without those inserts. The banner stays until the plugins are
 * restored or the user explicitly dismisses it, and Rescan restores
 * newly-installed plugins in place — saved knob state included —
 * without reloading the project.
 */
export function MissingPluginsBanner() {
  const missing = useMissingPluginsStore(s => s.missing)
  const dismissed = useMissingPluginsStore(s => s.dismissed)
  const setMissing = useMissingPluginsStore(s => s.set)
  const dismiss = useMissingPluginsStore(s => s.dismiss)
  const [expanded, setExpanded] = useState(false)
  const [busy, setBusy] = useState(false)

  if (missing.length === 0 || dismissed) return null

  const byPlugin = new Map<string, MissingPluginInfo[]>()
  for (const m of missing) {
    byPlugin.set(m.pluginId, [...(byPlugin.get(m.pluginId) ?? []), m])
  }

  const rescan = async () => {
    setBusy(true)
    const before = missing.length
    try {
      const still = await invoke<MissingPluginInfo[]>('rescan_and_restore_missing_plugins')
      setMissing(still)
      const restored = before - still.length
      const push = useNotificationStore.getState().push
      if (restored > 0 && still.length === 0) {
        push('info', `All ${restored} missing plugin instance${restored === 1 ? '' : 's'} restored — saved settings intact.`)
      } else if (restored > 0) {
        push('info', `${restored} restored, ${still.length} still missing.`)
      } else {
        push('warning', 'Rescan finished — the missing plugins are still not installed.', {
          detail: 'Install them (or add their folder under Audio settings → plugin paths), then rescan again.',
        })
      }
    } catch (e) {
      useNotificationStore.getState().push('error', 'Plugin rescan failed', { detail: String(e) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <div
      data-testid="missing-plugins-banner"
      style={{
        display: 'flex', flexDirection: 'column',
        background: 'rgba(245,158,11,0.10)',
        borderBottom: '1px solid rgba(245,158,11,0.4)',
        fontSize: 12, color: hw.textPrimary, flexShrink: 0,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '5px 12px' }}>
        <span style={{ color: hw.yellow }}>⚠</span>
        <span style={{ flex: 1 }}>
          {missing.length} plugin instance{missing.length === 1 ? '' : 's'} missing — those inserts are
          bypassed, so the mix may sound different. State is preserved.
        </span>
        <button onClick={() => setExpanded(v => !v)} style={linkBtn}>
          {expanded ? 'Hide details' : 'Details'}
        </button>
        <button onClick={() => void rescan()} disabled={busy} style={{ ...linkBtn, fontWeight: 700 }}>
          {busy ? 'Rescanning…' : 'Rescan plugins'}
        </button>
        <button onClick={dismiss} title="Hide for this session" style={{ ...linkBtn, color: hw.textFaint }}>
          ×
        </button>
      </div>
      {expanded && (
        <div style={{ padding: '0 12px 7px 34px', color: hw.textSecondary, fontSize: 11.5 }}>
          {[...byPlugin.entries()].map(([pid, uses]) => (
            <div key={pid} style={{ margin: '2px 0' }}>
              <span style={{ fontFamily: 'ui-monospace, Menlo, monospace' }}>{pid}</span>
              {' — '}
              {uses.map(u => `${u.trackName} · slot #${u.slotIndex + 1}`).join(', ')}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

const linkBtn: CSSProperties = {
  background: 'transparent', border: 'none', cursor: 'pointer',
  color: '#F59E0B', fontSize: 12, padding: '2px 6px',
}
