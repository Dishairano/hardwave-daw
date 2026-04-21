import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { hw } from '../theme'

export type TemplateId = 'blank' | 'beat4' | 'vocal' | 'mixing'

interface TemplateDef {
  id: TemplateId
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
  const [hover, setHover] = useState<TemplateId | null>(null)
  const firstBtn = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    firstBtn.current?.focus()
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onCancel])

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
        width: 520,
        maxWidth: '90vw',
        boxShadow: '0 10px 40px rgba(0,0,0,0.6)',
      }}>
        <div style={{ fontSize: 15, fontWeight: 600, color: hw.textPrimary, marginBottom: 4 }}>
          New project
        </div>
        <div style={{ fontSize: 12, color: hw.textMuted, marginBottom: 16 }}>
          Choose a template to start from.
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10, marginBottom: 18 }}>
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
