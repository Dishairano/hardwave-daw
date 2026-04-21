import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { hw } from '../theme'
import { useUserTemplateStore } from '../stores/userTemplateStore'

export type BuiltInTemplateId = 'blank' | 'beat4' | 'vocal' | 'mixing'
export type TemplateId = BuiltInTemplateId | `user:${string}`

interface TemplateDef {
  id: BuiltInTemplateId
  title: string
  subtitle: string
  detail: string
}

export const TEMPLATES: TemplateDef[] = [
  { id: 'blank', title: 'Blank', subtitle: 'Empty project', detail: 'Start from scratch with no tracks.' },
  { id: 'beat4', title: '4-Track Beat', subtitle: 'Drums + bass', detail: 'Kick, Snare, Hi-Hat, Bass — ready for beat-making.' },
  { id: 'vocal', title: 'Vocal Session', subtitle: 'Lead + backing + FX', detail: 'Vocal, Backing Vocals, FX Return — ready for tracking.' },
  { id: 'mixing', title: 'Mixing', subtitle: '8 tracks, pre-routed', detail: 'Eight audio tracks, Bus A/B return sends, Master.' },
]

interface Props {
  onPick: (id: TemplateId) => void
  onCancel: () => void
}

export function TemplateDialog({ onPick, onCancel }: Props) {
  const [hover, setHover] = useState<string | null>(null)
  const firstBtn = useRef<HTMLButtonElement>(null)
  const userTemplates = useUserTemplateStore(s => s.templates)
  const loadUserTemplates = useUserTemplateStore(s => s.load)
  const removeUserTemplate = useUserTemplateStore(s => s.remove)
  const renameUserTemplate = useUserTemplateStore(s => s.rename)

  useEffect(() => { loadUserTemplates() }, [loadUserTemplates])

  useEffect(() => {
    firstBtn.current?.focus()
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onCancel])

  const handleRename = (id: string, current: string) => {
    const next = window.prompt('Rename template', current)
    if (next !== null) renameUserTemplate(id, next)
  }

  const handleDelete = (id: string, name: string) => {
    if (window.confirm(`Delete template "${name}"?`)) removeUserTemplate(id)
  }

  return createPortal(
    <div style={{
      position: 'fixed', inset: 0, zIndex: 10000,
      background: 'rgba(0,0,0,0.6)',
      display: 'flex', alignItems: 'center', justifyContent: 'center',
    }}>
      <div style={{
        background: hw.bg,
        border: `1px solid ${hw.border}`,
        borderRadius: hw.radius.lg,
        padding: 20,
        width: 560,
        maxWidth: '90vw',
        maxHeight: '90vh',
        overflowY: 'auto',
        boxShadow: '0 10px 40px rgba(0,0,0,0.6)',
      }}>
        <div style={{ fontSize: 15, fontWeight: 600, color: hw.textPrimary, marginBottom: 4 }}>
          New project
        </div>
        <div style={{ fontSize: 12, color: hw.textMuted, marginBottom: 16 }}>
          Choose a template to start from.
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10, marginBottom: 14 }}>
          {TEMPLATES.map((t, i) => {
            const active = hover === t.id
            return (
              <button
                key={t.id}
                ref={i === 0 ? firstBtn : undefined}
                onMouseEnter={() => setHover(t.id)}
                onMouseLeave={() => setHover(null)}
                onClick={() => onPick(t.id)}
                style={{
                  textAlign: 'left',
                  padding: '12px 14px',
                  background: active ? hw.bgElevated : hw.bgSurface,
                  border: `1px solid ${active ? hw.accent : hw.border}`,
                  borderRadius: hw.radius.md,
                  color: hw.textPrimary,
                  cursor: 'pointer',
                  transition: 'border-color 0.1s, background 0.1s',
                }}
              >
                <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 2 }}>{t.title}</div>
                <div style={{ fontSize: 11, color: hw.textMuted, marginBottom: 6 }}>{t.subtitle}</div>
                <div style={{ fontSize: 11, color: hw.textSecondary, lineHeight: 1.4 }}>{t.detail}</div>
              </button>
            )
          })}
        </div>

        {userTemplates.length > 0 && (
          <>
            <div style={{
              fontSize: 10, fontWeight: 600, color: hw.textFaint,
              textTransform: 'uppercase', letterSpacing: 0.6, margin: '4px 0 8px',
            }}>
              Your templates
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10, marginBottom: 18 }}>
              {userTemplates.map(t => {
                const active = hover === t.id
                return (
                  <div
                    key={t.id}
                    onMouseEnter={() => setHover(t.id)}
                    onMouseLeave={() => setHover(null)}
                    style={{
                      position: 'relative',
                      background: active ? hw.bgElevated : hw.bgSurface,
                      border: `1px solid ${active ? hw.accent : hw.border}`,
                      borderRadius: hw.radius.md,
                      transition: 'border-color 0.1s, background 0.1s',
                    }}
                  >
                    <button
                      onClick={() => onPick(`user:${t.id}`)}
                      style={{
                        width: '100%',
                        textAlign: 'left',
                        padding: '12px 14px',
                        background: 'transparent',
                        border: 'none',
                        color: hw.textPrimary,
                        cursor: 'pointer',
                      }}
                    >
                      <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 2 }}>{t.name}</div>
                      <div style={{ fontSize: 11, color: hw.textMuted, marginBottom: 6 }}>
                        {t.tracks.length} track{t.tracks.length === 1 ? '' : 's'}
                      </div>
                      <div style={{ fontSize: 11, color: hw.textSecondary, lineHeight: 1.4 }}>
                        {t.tracks.slice(0, 4).map(x => x.name).join(', ')}
                        {t.tracks.length > 4 ? '…' : ''}
                      </div>
                    </button>
                    <div style={{ position: 'absolute', top: 6, right: 6, display: 'flex', gap: 4 }}>
                      <button
                        title="Rename"
                        onClick={e => { e.stopPropagation(); handleRename(t.id, t.name) }}
                        style={utBtnStyle}
                      >✎</button>
                      <button
                        title="Delete"
                        onClick={e => { e.stopPropagation(); handleDelete(t.id, t.name) }}
                        style={{ ...utBtnStyle, color: hw.red }}
                      >×</button>
                    </div>
                  </div>
                )
              })}
            </div>
          </>
        )}

        <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
          <button
            onClick={onCancel}
            style={{
              padding: '8px 16px',
              fontSize: 13,
              color: hw.textPrimary,
              background: hw.bgElevated,
              border: `1px solid ${hw.border}`,
              borderRadius: hw.radius.md,
              cursor: 'pointer',
              minWidth: 90,
            }}
          >
            Cancel
          </button>
        </div>
      </div>
    </div>,
    document.body,
  )
}

const utBtnStyle: React.CSSProperties = {
  width: 20, height: 20,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 13, fontWeight: 500,
  color: hw.textMuted,
  background: 'rgba(0,0,0,0.45)',
  border: `1px solid ${hw.border}`,
  borderRadius: 4,
  padding: 0,
  cursor: 'pointer',
  lineHeight: 1,
}
