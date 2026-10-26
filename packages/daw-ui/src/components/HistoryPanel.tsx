import { useMemo } from 'react'
import { createPortal } from 'react-dom'
import { hw } from '../theme'
import { useHistoryStore } from '../stores/historyStore'
import { useTrackStore } from '../stores/trackStore'
import { DialogFrame } from './ui/DialogFrame'
import { t, tf, useLanguage } from '../i18n'

interface Props {
  onClose: () => void
}

function formatTime(t: number) {
  const d = new Date(t)
  const hh = String(d.getHours()).padStart(2, '0')
  const mm = String(d.getMinutes()).padStart(2, '0')
  const ss = String(d.getSeconds()).padStart(2, '0')
  return `${hh}:${mm}:${ss}`
}

export function HistoryPanel({ onClose }: Props) {
  useLanguage()
  const entries = useHistoryStore(s => s.entries)
  const cursor = useHistoryStore(s => s.cursor)
  const jumpTo = useHistoryStore(s => s.jumpTo)
  const clear = useHistoryStore(s => s.clear)
  const undo = useTrackStore(s => s.undo)
  const redo = useTrackStore(s => s.redo)

  const rows = useMemo(() => {
    const withInitial = [{ id: '__initial__', label: 'Project opened', time: 0 } as { id: string; label: string; time: number }, ...entries]
    return withInitial
  }, [entries])

  const handleJump = async (targetCursor: number) => {
    await jumpTo(targetCursor, undo, redo)
  }

  return createPortal(
    <DialogFrame
      title={t('History')}
      subtitle={entries.length === 1
        ? tf('1 action · cursor at {cursor}', { cursor })
        : tf('{n} actions · cursor at {cursor}', { n: entries.length, cursor })}
      onClose={onClose}
      width={520}
      headerActions={<>
          <button
            onClick={() => clear()}
            disabled={entries.length === 0}
            style={{
              padding: '3px 10px', fontSize: 12, background: 'transparent',
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
              color: entries.length === 0 ? hw.textFaint : hw.textSecondary,
              cursor: entries.length === 0 ? 'default' : 'pointer',
            }}
          >{t('Clear log')}</button>
      </>}
    >
      <div style={{ flex: 1, overflowY: 'auto', padding: 4 }}>
        {rows.map((e, i) => {
          const targetCursor = i
          const isCurrent = targetCursor === cursor
          const isApplied = targetCursor <= cursor
          return (
            <button
              key={e.id}
              onClick={() => handleJump(targetCursor)}
              title={
                isCurrent ? t('Current state') :
                targetCursor < cursor
                  ? (cursor - targetCursor === 1 ? t('Undo 1 step') : tf('Undo {n} steps', { n: cursor - targetCursor }))
                  : (targetCursor - cursor === 1 ? t('Redo 1 step') : tf('Redo {n} steps', { n: targetCursor - cursor }))
              }
              style={{
                display: 'flex', alignItems: 'center', gap: 10,
                width: '100%', padding: '6px 10px', textAlign: 'left',
                background: isCurrent ? `${hw.accent}22` : 'transparent',
                border: `1px solid ${isCurrent ? hw.accent : 'transparent'}`,
                borderRadius: hw.radius.sm,
                color: isApplied ? hw.textPrimary : hw.textFaint,
                cursor: isCurrent ? 'default' : 'pointer',
                marginBottom: 2,
              }}
            >
              <span style={{
                width: 16, textAlign: 'center', fontSize: 12,
                color: isCurrent ? hw.accent : hw.textFaint,
              }}>
                {isCurrent ? '▶' : isApplied ? '·' : '○'}
              </span>
              <span style={{ flex: 1, fontSize: 12.5, fontStyle: e.id === '__initial__' ? 'italic' : 'normal' }}>
                {t(e.label)}
              </span>
              {e.time > 0 && (
                <span style={{ fontSize: 11, color: hw.textFaint, fontVariantNumeric: 'tabular-nums' }}>
                  {formatTime(e.time)}
                </span>
              )}
            </button>
          )
        })}
      </div>

      <div style={{
        padding: '6px 12px', borderTop: `1px solid ${hw.border}`,
        background: hw.bgElevated, fontSize: 11, color: hw.textFaint,
      }}>
        {t('Click any row to undo or redo to that state. Tracks volume/pan, clip edits, track add/remove, and more.')}
      </div>
    </DialogFrame>,
    document.body,
  )
}
