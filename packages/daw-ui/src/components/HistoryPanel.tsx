import { useMemo } from 'react'
import { createPortal } from 'react-dom'
import { hw } from '../theme'
import { useHistoryStore } from '../stores/historyStore'
import { useTrackStore } from '../stores/trackStore'
import { DialogFrame } from './ui/DialogFrame'

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
      title="History"
      subtitle={`${entries.length} action${entries.length === 1 ? '' : 's'} · cursor at ${cursor}`}
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
          >Clear log</button>
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
                isCurrent ? 'Current state' :
                targetCursor < cursor ? `Undo ${cursor - targetCursor} step${cursor - targetCursor === 1 ? '' : 's'}` :
                `Redo ${targetCursor - cursor} step${targetCursor - cursor === 1 ? '' : 's'}`
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
                {e.label}
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
        Click any row to undo or redo to that state. Tracks volume/pan, clip edits, track add/remove, and more.
      </div>
    </DialogFrame>,
    document.body,
  )
}
