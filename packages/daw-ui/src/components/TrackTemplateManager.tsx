import { useState } from 'react'
import { hw } from '../theme'
import { useTrackTemplateStore } from '../stores/trackTemplateStore'
import { useNotificationStore } from '../stores/notificationStore'

export function TrackTemplateManager({ open, onClose }: { open: boolean; onClose: () => void }) {
  const templates = useTrackTemplateStore(s => s.templates)
  const rename = useTrackTemplateStore(s => s.rename)
  const remove = useTrackTemplateStore(s => s.remove)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')

  if (!open) return null

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
              key={t.id}
              style={{
                display: 'flex', alignItems: 'center', gap: 10,
                padding: '6px 10px', borderRadius: hw.radius.sm,
                background: 'rgba(255,255,255,0.02)',
                border: `1px solid ${hw.border}`,
                marginBottom: 4,
              }}
            >
              <div style={{
                width: 4, height: 28, background: t.color, borderRadius: 2, flexShrink: 0,
              }} />
              <div style={{ flex: 1, minWidth: 0 }}>
                {editingId === t.id ? (
                  <input
                    autoFocus
                    value={draft}
                    onChange={e => setDraft(e.target.value)}
                    onBlur={() => {
                      const trimmed = draft.trim()
                      if (trimmed && trimmed !== t.name) rename(t.id, trimmed)
                      setEditingId(null)
                    }}
                    onKeyDown={e => {
                      if (e.key === 'Enter') {
                        const trimmed = draft.trim()
                        if (trimmed && trimmed !== t.name) rename(t.id, trimmed)
                        setEditingId(null)
                      } else if (e.key === 'Escape') {
                        setEditingId(null)
                      }
                    }}
                    style={{
                      width: '100%', fontSize: 11, color: hw.textPrimary,
                      background: hw.bgInput,
                      border: `1px solid ${hw.accent}`, borderRadius: hw.radius.sm,
                      padding: '2px 6px', outline: 'none',
                    }}
                  />
                ) : (
                  <div
                    onDoubleClick={() => { setEditingId(t.id); setDraft(t.name) }}
                    style={{
                      fontSize: 12, fontWeight: 500, color: hw.textPrimary,
                      overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                      cursor: 'text',
                    }}
                  >
                    {t.name}
                  </div>
                )}
                <div style={{ fontSize: 9, color: hw.textFaint, marginTop: 2 }}>
                  {t.kind} · {t.volumeDb.toFixed(1)} dB · pan {t.pan >= 0 ? `+${t.pan.toFixed(2)}` : t.pan.toFixed(2)}
                </div>
              </div>
              <button
                onClick={() => {
                  remove(t.id)
                  useNotificationStore.getState().push('info', `Deleted template "${t.name}"`)
                }}
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
