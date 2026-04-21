import { useEffect, useRef, useState } from 'react'
import { hw } from '../../theme'
import { useSendStore, type SendInfo } from '../../stores/sendStore'
import { useTrackStore } from '../../stores/trackStore'

interface SendsEditorProps {
  trackId: string
}

export function SendsEditor({ trackId }: SendsEditorProps) {
  const { byTrack, fetchAll, addSend, removeSend, setGain, setPreFader, setEnabled, createReturnWithSend } = useSendStore()
  const { tracks } = useTrackStore()
  const [targetPicker, setTargetPicker] = useState(false)
  const sends = byTrack[trackId] ?? []

  useEffect(() => { fetchAll() }, [])

  const trackById = new Map(tracks.map(t => [t.id, t]))
  const selfExists = tracks.some(t => t.id === trackId)
  if (!selfExists) return null

  // Every other non-master track is a legal target. (Cycle prevention is
  // enforced server-side; we show all, the command will reject cycles.)
  const candidates = tracks.filter(t => t.id !== trackId && t.kind !== 'Master')

  const handleAdd = async (targetId: string) => {
    setTargetPicker(false)
    try {
      await addSend(trackId, targetId, 0, false)
    } catch (err) {
      console.warn('add_send failed:', err)
    }
  }

  const handleCreateReturn = async (kind: 'reverb' | 'delay') => {
    setTargetPicker(false)
    try {
      await createReturnWithSend(trackId, kind === 'reverb' ? 'Reverb Return' : 'Delay Return')
    } catch (err) {
      console.warn('create_return_with_send failed:', err)
    }
  }

  return (
    <div data-testid={`sends-${trackId}`} style={{
      borderTop: `1px solid ${hw.border}`,
      padding: '3px 3px 3px', display: 'flex', flexDirection: 'column', gap: 2,
      background: 'rgba(255,255,255,0.015)',
    }}>
      <div style={{
        display: 'flex', alignItems: 'center', gap: 3, fontSize: 7,
        color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase',
      }}>
        <span style={{ flex: 1 }}>Sends</span>
        <button
          onClick={() => setTargetPicker(v => !v)}
          title="Add send"
          data-testid={`sends-add-${trackId}`}
          style={{
            width: 14, height: 14, padding: 0, fontSize: 10, lineHeight: 1,
            color: hw.textMuted, background: 'rgba(255,255,255,0.04)',
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
            cursor: 'pointer',
          }}
        >+</button>
      </div>

      {sends.length === 0 && !targetPicker && (
        <div style={{ fontSize: 8, color: hw.textFaint, textAlign: 'center', padding: '2px 0' }}>
          none
        </div>
      )}

      {sends.map((s) => {
        const target = trackById.get(s.target)
        const targetName = target?.name ?? s.target.slice(0, 6)
        const targetColor = target?.color ?? hw.border
        return (
          <SendRow
            key={`${trackId}-${s.index}`}
            send={s}
            targetName={targetName}
            targetColor={targetColor}
            onGain={v => setGain(trackId, s.index, v)}
            onTogglePreFader={() => setPreFader(trackId, s.index, !s.preFader)}
            onToggleEnabled={() => setEnabled(trackId, s.index, !s.enabled)}
            onRemove={() => removeSend(trackId, s.index)}
          />
        )
      })}

      {targetPicker && (
        <div data-testid={`sends-picker-${trackId}`} style={{
          position: 'absolute', zIndex: 50,
          left: 2, right: 2, top: 'calc(100% + 2px)',
          maxHeight: 180, overflowY: 'auto',
          background: 'rgba(12,12,18,0.97)',
          border: `1px solid ${hw.borderLight}`,
          borderRadius: hw.radius.md,
          boxShadow: '0 6px 24px rgba(0,0,0,0.6)',
          padding: 4,
        }}>
          <div style={{ fontSize: 8, color: hw.textFaint, padding: '3px 6px 4px', letterSpacing: 0.5, textTransform: 'uppercase' }}>Send to</div>
          <button onClick={() => handleCreateReturn('reverb')} style={pickerItemStyle}>+ New Reverb Return</button>
          <button onClick={() => handleCreateReturn('delay')} style={pickerItemStyle}>+ New Delay Return</button>
          <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
          {candidates.length === 0 ? (
            <div style={{ fontSize: 9, color: hw.textFaint, padding: '3px 6px' }}>
              No other tracks available
            </div>
          ) : candidates.map(t => (
            <button
              key={t.id}
              onClick={() => handleAdd(t.id)}
              style={pickerItemStyle}
            >
              <span style={{ width: 6, height: 6, borderRadius: 1, background: t.color }} />
              <span style={{ flex: 1, textAlign: 'left', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{t.name}</span>
              <span style={{ fontSize: 8, color: hw.textFaint }}>{t.kind}</span>
            </button>
          ))}
          <button
            onClick={() => setTargetPicker(false)}
            style={{ ...pickerItemStyle, color: hw.textFaint }}
          >Cancel</button>
        </div>
      )}
    </div>
  )
}

interface SendRowProps {
  send: SendInfo
  targetName: string
  targetColor: string
  onGain: (db: number) => void
  onTogglePreFader: () => void
  onToggleEnabled: () => void
  onRemove: () => void
}

function SendRow({ send, targetName, targetColor, onGain, onTogglePreFader, onToggleEnabled, onRemove }: SendRowProps) {
  const faderRef = useRef<HTMLInputElement>(null)

  // Debounce the drag so we don't spam the backend / history stack.
  const [draft, setDraft] = useState(send.gainDb)
  useEffect(() => { setDraft(send.gainDb) }, [send.gainDb])

  return (
    <div
      data-testid={`send-row-${send.index}`}
      title={`Send → ${targetName} (${send.preFader ? 'pre' : 'post'}-fader)`}
      style={{
        display: 'flex', flexDirection: 'column', gap: 1,
        padding: 3,
        border: `1px solid ${send.enabled ? hw.border : hw.borderDark}`,
        borderLeft: `2px solid ${targetColor}`,
        borderRadius: hw.radius.sm,
        background: send.enabled ? 'rgba(255,255,255,0.03)' : 'rgba(255,255,255,0.01)',
        opacity: send.enabled ? 1 : 0.55,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 2 }}>
        <span
          style={{
            flex: 1, fontSize: 8, color: hw.textPrimary,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}
        >→ {targetName}</span>
        <button
          onClick={onTogglePreFader}
          title={send.preFader ? 'Pre-fader (click for post)' : 'Post-fader (click for pre)'}
          style={{
            fontSize: 7, padding: '1px 3px', height: 12,
            color: send.preFader ? hw.accent : hw.textMuted,
            background: send.preFader ? hw.redDim : 'rgba(255,255,255,0.04)',
            border: `1px solid ${hw.border}`, borderRadius: 3, cursor: 'pointer',
          }}
        >{send.preFader ? 'PRE' : 'POST'}</button>
        <button
          onClick={onToggleEnabled}
          title={send.enabled ? 'Disable send' : 'Enable send'}
          style={{
            fontSize: 7, padding: '1px 3px', height: 12,
            color: send.enabled ? hw.green : hw.textFaint,
            background: 'rgba(255,255,255,0.04)',
            border: `1px solid ${hw.border}`, borderRadius: 3, cursor: 'pointer',
          }}
        >{send.enabled ? 'ON' : 'OFF'}</button>
        <button
          onClick={onRemove}
          title="Remove send"
          style={{
            width: 12, height: 12, padding: 0, fontSize: 9, lineHeight: 1,
            color: hw.textMuted, background: 'rgba(255,255,255,0.04)',
            border: `1px solid ${hw.border}`, borderRadius: 3, cursor: 'pointer',
          }}
        >×</button>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
        <input
          ref={faderRef}
          type="range"
          min={-60}
          max={6}
          step={0.1}
          value={draft}
          onChange={e => setDraft(parseFloat(e.target.value))}
          onMouseUp={() => onGain(draft)}
          onKeyUp={() => onGain(draft)}
          onDoubleClick={() => { setDraft(0); onGain(0) }}
          style={{ flex: 1, accentColor: hw.accent }}
        />
        <span style={{
          fontSize: 8, color: hw.textPrimary, fontFamily: "'Consolas', monospace",
          minWidth: 26, textAlign: 'right',
        }}>
          {draft <= -60 ? '-∞' : `${draft > 0 ? '+' : ''}${draft.toFixed(1)}`}
        </span>
      </div>
    </div>
  )
}

const pickerItemStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center', gap: 4, width: '100%',
  padding: '4px 6px', fontSize: 9, color: hw.textSecondary,
  background: 'transparent', border: 'none', borderRadius: 3,
  cursor: 'pointer', textAlign: 'left',
}
