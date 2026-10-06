import { useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'
import { useNotificationStore } from '../../stores/notificationStore'

/**
 * Tuning a sung take, note by note.
 *
 * Each note is moved by its own amount, which is the point: a singer
 * is sharp on one word and flat on the next. Strength decides how far
 * towards the note it goes, because hard dance wants all the way and a
 * sung chorus usually does not. The original take is kept: the tuned
 * audio is a new file and the clip is pointed at it.
 */

const ROOTS = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']

export function TuneDialog({
  trackId, clipId, onClose,
}: { trackId: string; clipId: string; onClose: () => void }) {
  const [strength, setStrength] = useState(1)
  const [root, setRoot] = useState(0)
  const [scale, setScale] = useState<'chromatic' | 'major' | 'minor'>('chromatic')
  const [tolerance, setTolerance] = useState(0)
  const [busy, setBusy] = useState(false)

  const apply = async () => {
    setBusy(true)
    try {
      const result = await invoke<{ notes: number; moved: number; worstCents: number; path: string }>(
        'tune_audio_clip',
        { trackId, clipId, strength, root, scale, ignoreWithinCents: tolerance },
      )
      await useTrackStore.getState().fetchTracks()
      useNotificationStore.getState().push('info',
        `${result.moved} of ${result.notes} ${result.notes === 1 ? 'note' : 'notes'} tuned`, {
          detail: `The take was up to ${Math.round(result.worstCents)} cents out. `
            + 'The original is untouched: the tuned audio is a new file.',
        })
      onClose()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not tune that', { detail: String(e) })
      setBusy(false)
    }
  }

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 9800,
        background: 'rgba(0,0,0,0.45)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div style={{
        width: 420, maxWidth: '94vw',
        background: hw.bg, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg, overflow: 'hidden',
      }}>
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 12,
          background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Tune this take</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>note by note, in place</div>
          <div style={{ flex: 1 }} />
          <button onClick={onClose} style={btn()}>Close</button>
        </div>

        <div style={{ padding: 14, display: 'flex', flexDirection: 'column', gap: 12 }}>
          <Row label="Strength" value={`${Math.round(strength * 100)}%`}>
            <input
              type="range" min={0} max={1} step={0.05} value={strength}
              onChange={e => setStrength(Number(e.target.value))}
              style={{ width: '100%' }}
            />
          </Row>
          <Row label="Key">
            <div style={{ display: 'flex', gap: 6 }}>
              <select
                value={root}
                onChange={e => setRoot(Number(e.target.value))}
                style={select()}
              >
                {ROOTS.map((name, i) => <option key={name} value={i}>{name}</option>)}
              </select>
              <select
                value={scale}
                onChange={e => setScale(e.target.value as 'chromatic' | 'major' | 'minor')}
                style={select()}
              >
                <option value="chromatic">any note</option>
                <option value="major">major</option>
                <option value="minor">minor</option>
              </select>
            </div>
          </Row>
          <Row label="Leave alone within" value={`${tolerance} cents`}>
            <input
              type="range" min={0} max={50} step={5} value={tolerance}
              onChange={e => setTolerance(Number(e.target.value))}
              style={{ width: '100%' }}
            />
          </Row>
          <div style={{ fontSize: 10, color: hw.textFaint, lineHeight: 1.5 }}>
            One voice at a time. The take keeps its length and its place, and the
            original file is left where it is.
          </div>
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
            <button onClick={onClose} style={btn()}>Cancel</button>
            <button
              onClick={() => void apply()}
              disabled={busy}
              style={{ ...btn(), background: hw.accent, color: '#fff' }}
            >{busy ? 'Tuning…' : 'Tune'}</button>
          </div>
        </div>
      </div>
    </div>
  )
}

function Row({ label, value, children }: { label: string; value?: string; children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11 }}>
        <span style={{ color: hw.textSecondary }}>{label}</span>
        {value && <span style={{ color: hw.textFaint }}>{value}</span>}
      </div>
      {children}
    </div>
  )
}

function btn(): React.CSSProperties {
  return {
    padding: '4px 10px', fontSize: 11, fontWeight: 600,
    background: 'rgba(255,255,255,0.08)', color: hw.textSecondary,
    border: 'none', borderRadius: hw.radius.sm, cursor: 'pointer', fontFamily: 'inherit',
  }
}

function select(): React.CSSProperties {
  return {
    padding: '3px 6px', fontSize: 11, fontFamily: 'inherit',
    background: 'rgba(255,255,255,0.04)', color: hw.textPrimary,
    border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
  }
}
