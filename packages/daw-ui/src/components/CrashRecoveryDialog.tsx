import { useEffect } from 'react'
import { createPortal } from 'react-dom'
import { hw } from '../theme'
import { t, tf, useLanguage } from '../i18n'

export type CrashChoice = 'recover' | 'discard' | 'open'

interface Props {
  autosavePath: string
  modifiedUnix: number
  onChoice: (c: CrashChoice) => void
}

export function CrashRecoveryDialog({ autosavePath, modifiedUnix, onChoice }: Props) {
  useLanguage()
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
    : t('unknown time')
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
          {t('Recover from crash?')}
        </div>
        <div style={{ fontSize: 13, color: hw.textSecondary, marginBottom: 6, lineHeight: 1.5 }}>
          {tf('It looks like the last session ended unexpectedly. An auto-save from {when} is available.', { when })}
        </div>
        <div style={{ fontSize: 11, color: hw.textFaint, marginBottom: 6, fontFamily: 'monospace' }}>
          {name}
        </div>
        <div style={{ fontSize: 11.5, color: hw.textFaint, marginBottom: 18, lineHeight: 1.5 }}>
          {withBold(
            t('{recover} loads that auto-save · {open} ignores it and picks a file · {discard} deletes it.'),
            { recover: t('Recover'), open: t('Open project…'), discard: t('Discard') },
          )}{' '}
          {t('If recovery fails, File → Revert to last backup has older snapshots.')}
        </div>
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button onClick={() => onChoice('discard')} style={btn(false)}>{t('Discard')}</button>
          <button onClick={() => onChoice('open')} style={btn(false)}>{t('Open project…')}</button>
          <button onClick={() => onChoice('recover')} style={btn(true)}>{t('Recover')}</button>
        </div>
      </div>
    </div>,
    document.body,
  )
}

/**
 * A translated sentence with the button names in bold where its {blanks}
 * are, so a translation can put them wherever its word order wants.
 */
function withBold(sentence: string, names: Record<string, string>) {
  return sentence.split(/(\{\w+\})/).map((part, i) => {
    const key = /^\{(\w+)\}$/.exec(part)?.[1]
    return key && key in names
      ? <b key={i} style={{ color: hw.textSecondary }}>{names[key]}</b>
      : part
  })
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
