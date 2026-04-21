import { useEffect, useRef, useState } from 'react'
import { hw } from '../theme'
import { useNotificationStore } from '../stores/notificationStore'

let clipboardValue: number | null = null

export function copyParameterValue(value: number) {
  clipboardValue = value
}

export function pasteParameterValue(): number | null {
  return clipboardValue
}

export interface ParameterContextMenuProps {
  x: number
  y: number
  label: string
  value: number
  defaultValue: number
  unit?: string
  min: number
  max: number
  decimals?: number
  onSet: (v: number) => void
  onClose: () => void
}

export function ParameterContextMenu(props: ParameterContextMenuProps) {
  const { x, y, label, value, defaultValue, unit = '', min, max, decimals = 2, onSet, onClose } = props
  const [typing, setTyping] = useState(false)
  const [draft, setDraft] = useState(() => value.toFixed(decimals))
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (typing) inputRef.current?.select()
  }, [typing])

  useEffect(() => {
    const close = (e: MouseEvent) => {
      const t = e.target as HTMLElement
      if (t.closest('[data-param-ctx-menu]')) return
      onClose()
    }
    window.addEventListener('mousedown', close)
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', close)
      window.removeEventListener('keydown', onKey)
    }
  }, [onClose])

  const clamp = (v: number) => Math.max(min, Math.min(max, v))

  const commitTyped = () => {
    const parsed = parseFloat(draft)
    if (Number.isFinite(parsed)) {
      onSet(clamp(parsed))
    }
    onClose()
  }

  return (
    <div
      data-param-ctx-menu
      onMouseDown={e => e.stopPropagation()}
      onContextMenu={e => e.preventDefault()}
      style={{
        position: 'fixed', left: x, top: y, zIndex: 16000,
        minWidth: 200, padding: 4,
        background: 'rgba(12,12,18,0.98)',
        border: `1px solid ${hw.borderLight}`,
        borderRadius: hw.radius.md,
        boxShadow: '0 8px 32px rgba(0,0,0,0.55)',
        backdropFilter: hw.blur.md,
      }}
    >
      <div style={{
        padding: '4px 8px 2px', fontSize: 8, color: hw.textFaint,
        letterSpacing: 0.5, textTransform: 'uppercase',
      }}>
        {label} — {value.toFixed(decimals)}{unit}
      </div>
      {typing ? (
        <div style={{ padding: '4px 6px' }}>
          <input
            ref={inputRef}
            value={draft}
            onChange={e => setDraft(e.target.value)}
            onBlur={commitTyped}
            onKeyDown={e => {
              if (e.key === 'Enter') commitTyped()
              else if (e.key === 'Escape') onClose()
            }}
            style={{
              width: '100%', fontSize: 11, color: hw.textPrimary,
              background: hw.bgInput,
              border: `1px solid ${hw.accent}`, borderRadius: hw.radius.sm,
              padding: '3px 6px', outline: 'none',
              fontFamily: 'ui-monospace, Menlo, monospace',
            }}
          />
          <div style={{ fontSize: 9, color: hw.textFaint, marginTop: 2 }}>
            Range: {min} to {max}{unit}
          </div>
        </div>
      ) : (
        <>
          <Item label="Reset to default" shortcut={defaultValue.toFixed(decimals) + unit} onClick={() => {
            onSet(defaultValue)
            onClose()
          }} />
          <Item label="Copy value" onClick={() => {
            copyParameterValue(value)
            useNotificationStore.getState().push('info', `Copied ${value.toFixed(decimals)}${unit}`)
            onClose()
          }} />
          <Item
            label="Paste value"
            disabled={clipboardValue == null}
            shortcut={clipboardValue != null ? clipboardValue.toFixed(decimals) + unit : undefined}
            onClick={() => {
              if (clipboardValue != null) onSet(clamp(clipboardValue))
              onClose()
            }}
          />
          <Item label="Type in value…" onClick={() => {
            setDraft(value.toFixed(decimals))
            setTyping(true)
          }} />
          <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
          <Item label="Automation…" disabled shortcut="soon" onClick={() => {}} />
          <Item label="MIDI Learn" disabled shortcut="soon" onClick={() => {}} />
        </>
      )}
    </div>
  )
}

function Item({ label, shortcut, disabled, onClick }: {
  label: string; shortcut?: string; disabled?: boolean; onClick: () => void
}) {
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      style={{
        width: '100%', display: 'flex', alignItems: 'center',
        padding: '5px 8px', gap: 8, border: 'none',
        background: 'transparent',
        color: disabled ? hw.textFaint : hw.textSecondary,
        fontSize: 11, cursor: disabled ? 'default' : 'pointer',
        borderRadius: hw.radius.sm,
        opacity: disabled ? 0.5 : 1,
      }}
      onMouseEnter={e => {
        if (disabled) return
        ;(e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.05)'
      }}
      onMouseLeave={e => {
        ;(e.currentTarget as HTMLElement).style.background = 'transparent'
      }}
    >
      <span style={{ flex: 1, textAlign: 'left' }}>{label}</span>
      {shortcut && <span style={{ fontSize: 9, color: hw.textFaint, fontFamily: 'ui-monospace, Menlo, monospace' }}>{shortcut}</span>}
    </button>
  )
}
