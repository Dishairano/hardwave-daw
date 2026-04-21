import { useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { hw } from '../theme'

export type SaveChangesChoice = 'save' | 'discard' | 'cancel'

interface Props {
  action: string
  onChoice: (c: SaveChangesChoice) => void
}

export function SaveChangesDialog({ action, onChoice }: Props) {
  const saveBtn = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    saveBtn.current?.focus()
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onChoice('cancel')
      if (e.key === 'Enter') onChoice('save')
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onChoice])

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
        minWidth: 380,
        maxWidth: 480,
        boxShadow: '0 10px 40px rgba(0,0,0,0.6)',
      }}>
        <div style={{
          fontSize: 14,
          fontWeight: 600,
          color: hw.textPrimary,
          marginBottom: 8,
        }}>
          Unsaved changes
        </div>
        <div style={{ fontSize: 13, color: hw.textSecondary, marginBottom: 20, lineHeight: 1.5 }}>
          You have unsaved changes in this project. {action}?
        </div>
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button
            onClick={() => onChoice('cancel')}
            style={btnStyle(false)}
          >
            Cancel
          </button>
          <button
            onClick={() => onChoice('discard')}
            style={btnStyle(false)}
          >
            Don't Save
          </button>
          <button
            ref={saveBtn}
            onClick={() => onChoice('save')}
            style={btnStyle(true)}
          >
            Save
          </button>
        </div>
      </div>
    </div>,
    document.body,
  )
}

function btnStyle(primary: boolean): React.CSSProperties {
  return {
    padding: '8px 16px',
    fontSize: 13,
    fontWeight: 500,
    color: primary ? hw.textBright : hw.textPrimary,
    background: primary ? hw.accent : hw.bgElevated,
    border: `1px solid ${primary ? hw.accent : hw.border}`,
    borderRadius: hw.radius.md,
    cursor: 'pointer',
    minWidth: 90,
  }
}
