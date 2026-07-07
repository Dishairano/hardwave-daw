import { useEffect } from 'react'
import { createPortal } from 'react-dom'
import { hw } from '../theme'

export type CrashChoice = 'recover' | 'discard' | 'open'

interface Props {
  autosavePath: string
  modifiedUnix: number
  onChoice: (c: CrashChoice) => void
}

export function CrashRecoveryDialog({ autosavePath, modifiedUnix, onChoice }: Props) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onChoice('discard')
      if (e.key === 'Enter') onChoice('recover')
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onChoice])

  const when = modifiedUnix > 0
    ? new Date(modifiedUnix * 1000).toLocaleString()
    : 'unknown time'
  const name = autosavePath.split(/[\\/]/).pop() || autosavePath

  return createPortal(
    <div style={{
      position: 'fixed', inset: 0, zIndex: 10000,
      background: 'rgba(0,0,0,0.7)',
      display: 'flex', alignItems: 'center', justifyContent: 'center',
    }}>
      <div style={{
        background: hw.bg,
        border: `1px solid ${hw.accent}`,
        borderRadius: hw.radius.lg,
        padding: 22,
        width: 460,
        maxWidth: '90vw',
        boxShadow: hw.glowRed,
      }}>
        <div style={{ fontSize: 15, fontWeight: 700, color: hw.textBright, marginBottom: 6 }}>
          Recover from crash?
        </div>
        <div style={{ fontSize: 13, color: hw.textSecondary, marginBottom: 6, lineHeight: 1.5 }}>
          It looks like the last session ended unexpectedly. An auto-save from {when} is available.
        </div>
        <div style={{ fontSize: 11, color: hw.textFaint, marginBottom: 6, fontFamily: 'monospace' }}>
          {name}
        </div>
        <div style={{ fontSize: 11.5, color: hw.textFaint, marginBottom: 18, lineHeight: 1.5 }}>
          <b style={{ color: hw.textSecondary }}>Recover</b> loads that auto-save ·{' '}
          <b style={{ color: hw.textSecondary }}>Open project…</b> ignores it and picks a file ·{' '}
          <b style={{ color: hw.textSecondary }}>Discard</b> deletes it.
          If recovery fails, File → Revert to last backup has older snapshots.
        </div>
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button onClick={() => onChoice('discard')} style={btn(false)}>Discard</button>
          <button onClick={() => onChoice('open')} style={btn(false)}>Open project…</button>
          <button onClick={() => onChoice('recover')} style={btn(true)}>Recover</button>
        </div>
      </div>
    </div>,
    document.body,
  )
}

function btn(primary: boolean): React.CSSProperties {
  return {
    padding: '8px 14px',
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
