import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useTrackStore } from '../stores/trackStore'
import { useNotificationStore } from '../stores/notificationStore'

interface TrackTemplateEntry {
  name: string
  /** "Audio" or "Midi". */
  kind: string
  plugins: number
}

/**
 * Track templates: a track kept with its plug-ins, their settings and the
 * bus and sends it feeds (right-click a track, Save as track template).
 * Templates could be saved before, but nothing ever added one back to a
 * song; here each one adds a track after the selected one.
 */
export function TrackTemplateManager({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [templates, setTemplates] = useState<TrackTemplateEntry[]>([])
  const reload = () => {
    invoke<TrackTemplateEntry[]>('list_track_templates').then(setTemplates).catch(() => setTemplates([]))
  }
  useEffect(() => { if (open) reload() }, [open])

  if (!open) return null
  const { push } = useNotificationStore.getState()

  const add = async (name: string) => {
    try {
      const afterTrackId = useTrackStore.getState().selectedTrackId ?? null
      await invoke<string>('add_track_from_template', { name, afterTrackId })
      await useTrackStore.getState().fetchTracks()
      push('info', `Added "${name}"`)
      onClose()
    } catch (err) {
      push('warning', 'The track was not added', { detail: String(err) })
    }
  }
  const remove = async (name: string) => {
    if (!window.confirm(`Delete the track template "${name}"?`)) return
    await invoke('delete_track_template', { name }).catch(() => {})
    reload()
  }

  return (
    <div
      onMouseDown={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 15000,
        background: 'rgba(0,0,0,0.55)', backdropFilter: hw.blur.sm,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div
        onMouseDown={e => e.stopPropagation()}
        style={{
          width: 'min(520px, 92vw)', maxHeight: '72vh',
          display: 'flex', flexDirection: 'column',
          background: 'rgba(12,12,18,0.98)', border: `1px solid ${hw.borderLight}`,
          borderRadius: hw.radius.lg, boxShadow: '0 16px 48px rgba(0,0,0,0.6)',
        }}
      >
        <div style={{
          padding: '12px 16px', borderBottom: `1px solid ${hw.border}`,
          display: 'flex', alignItems: 'center',
        }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: hw.textPrimary, marginRight: 'auto' }}>
            Track templates
          </div>
          <button
            onClick={onClose}
            style={{
              padding: '3px 10px', fontSize: 10, fontWeight: 600,
              color: hw.textMuted, background: hw.bgInput,
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, cursor: 'pointer',
            }}
          >Close</button>
        </div>

        <div style={{ overflowY: 'auto', padding: 12 }}>
          {templates.length === 0 && (
            <div style={{ padding: 24, textAlign: 'center', color: hw.textFaint, fontSize: 11 }}>
              No templates saved yet. Right-click a track and choose "Save as track template…".
            </div>
          )}
          {templates.map(t => (
            <div
              key={t.name}
              style={{
                display: 'flex', alignItems: 'center', gap: 10,
                padding: '6px 10px', borderRadius: hw.radius.sm,
                background: 'rgba(255,255,255,0.02)',
                border: `1px solid ${hw.border}`,
                marginBottom: 4,
              }}
            >
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{
                  fontSize: 12, fontWeight: 500, color: hw.textPrimary,
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                }}>
                  {t.name}
                </div>
                <div style={{ fontSize: 9, color: hw.textFaint, marginTop: 2 }}>
                  {t.kind === 'Midi' ? 'Instrument track' : 'Audio track'} · {t.plugins} plug-in{t.plugins === 1 ? '' : 's'}
                </div>
              </div>
              <button
                onClick={() => { void add(t.name) }}
                style={{
                  padding: '3px 8px', fontSize: 10, fontWeight: 600,
                  color: '#fff', background: hw.accent,
                  border: `1px solid ${hw.accent}`, borderRadius: hw.radius.sm, cursor: 'pointer',
                }}
              >Add track</button>
              <button
                onClick={() => { void remove(t.name) }}
                style={{
                  padding: '3px 8px', fontSize: 10, fontWeight: 600,
                  color: hw.red, background: hw.redDim,
                  border: `1px solid ${hw.red}40`, borderRadius: hw.radius.sm, cursor: 'pointer',
                }}
              >Delete</button>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}
