import { useMemo } from 'react'
import { createPortal } from 'react-dom'
import { hw } from '../theme'
import { useHistoryStore } from '../stores/historyStore'
import { useTrackStore } from '../stores/trackStore'

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
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 9700,
        background: 'rgba(0,0,0,0.45)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div
        style={{
          width: 520, maxWidth: '92vw', maxHeight: '80vh',
          display: 'flex', flexDirection: 'column',
          background: hw.bg, color: hw.textPrimary,
          border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg,
          overflow: 'hidden',
        }}
      >
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 12,
          background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>History</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>
            {entries.length} action{entries.length === 1 ? '' : 's'} · cursor at {cursor}
          </div>
          <div style={{ flex: 1 }} />
          <button
            onClick={() => clear()}
            disabled={entries.length === 0}
            style={{
              padding: '3px 10px', fontSize: 10, background: 'transparent',
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
              color: entries.length === 0 ? hw.textFaint : hw.textSecondary,
              cursor: entries.length === 0 ? 'default' : 'pointer',
            }}
          >Clear log</button>
          <button
            onClick={onClose}
            style={{
              padding: '3px 10px', fontSize: 10, background: 'transparent',
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
              color: hw.textSecondary, cursor: 'pointer',
            }}
          >Close</button>
        </div>

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
                  width: 16, textAlign: 'center', fontSize: 10,
                  color: isCurrent ? hw.accent : hw.textFaint,
                }}>
                  {isCurrent ? '▶' : isApplied ? '·' : '○'}
                </span>
                <span style={{ flex: 1, fontSize: 11, fontStyle: e.id === '__initial__' ? 'italic' : 'normal' }}>
                  {e.label}
                </span>
                {e.time > 0 && (
                  <span style={{ fontSize: 9, color: hw.textFaint, fontVariantNumeric: 'tabular-nums' }}>
                    {formatTime(e.time)}
                  </span>
                )}
              </button>
            )
          })}
        </div>

        <div style={{
          padding: '6px 12px', borderTop: `1px solid ${hw.border}`,
          background: hw.bgElevated, fontSize: 9, color: hw.textFaint,
        }}>
          Click any row to undo or redo to that state. Tracks volume/pan, clip edits, track add/remove, and more.
        </div>
      </div>
    </div>,
    document.body,
  )
}
