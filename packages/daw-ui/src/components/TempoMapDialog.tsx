import { useEffect, useState, useCallback } from 'react'
import { createPortal } from 'react-dom'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'

export interface TempoEntryInfo {
  tick: number
  bpm: number
  timeSigNum: number
  timeSigDen: number
  ramp: string
}

const PPQ = 960

function tickToBar(tick: number, tsNum: number): string {
  const beats = tick / PPQ
  const bar = Math.floor(beats / tsNum) + 1
  const beat = (beats % tsNum) + 1
  return `${bar}.${beat.toFixed(2)}`
}

interface Props {
  onClose: () => void
}

export function TempoMapDialog({ onClose }: Props) {
  const [entries, setEntries] = useState<TempoEntryInfo[]>([])
  const [err, setErr] = useState<string | null>(null)
  const [newTickBeats, setNewTickBeats] = useState<string>('16')
  const [newBpm, setNewBpm] = useState<string>('140')

  const reload = useCallback(async () => {
    try {
      const list = await invoke<TempoEntryInfo[]>('get_tempo_entries')
      setEntries(list)
      setErr(null)
    } catch (e) {
      setErr(String(e))
    }
  }, [])

  useEffect(() => { reload() }, [reload])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const handleAdd = async () => {
    const beats = parseFloat(newTickBeats)
    const bpm = parseFloat(newBpm)
    if (!isFinite(beats) || beats <= 0) { setErr('Position must be > 0 beats'); return }
    if (!isFinite(bpm)) { setErr('BPM must be a number'); return }
    try {
      await invoke('add_tempo_entry', { tick: Math.round(beats * PPQ), bpm, ramp: 'instant' })
      await reload()
    } catch (e) { setErr(String(e)) }
  }

  const handleRemove = async (index: number) => {
    try {
      await invoke('remove_tempo_entry', { index })
      await reload()
    } catch (e) { setErr(String(e)) }
  }

  const handleEdit = async (index: number, field: 'tick' | 'bpm' | 'ramp', value: string) => {
    const cur = entries[index]
    if (!cur) return
    const tick = field === 'tick' ? Math.max(0, Math.round(parseFloat(value) * PPQ)) : cur.tick
    const bpm = field === 'bpm' ? parseFloat(value) : cur.bpm
    const ramp = field === 'ramp' ? value : cur.ramp
    if (!isFinite(bpm)) { setErr('BPM must be a number'); return }
    try {
      await invoke('set_tempo_entry', { index, tick, bpm, ramp })
      await reload()
    } catch (e) { setErr(String(e)) }
  }

  const headerStyle: React.CSSProperties = {
    fontSize: 10, fontWeight: 600, color: hw.textFaint,
    textTransform: 'uppercase', letterSpacing: 0.6, padding: '4px 8px',
    borderBottom: `1px solid ${hw.border}`,
  }

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
        padding: 20, width: 620, maxWidth: '92vw', maxHeight: '90vh', overflowY: 'auto',
        boxShadow: '0 10px 40px rgba(0,0,0,0.6)',
      }}>
        <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', marginBottom: 4 }}>
          <div style={{ fontSize: 15, fontWeight: 600, color: hw.textPrimary }}>Tempo map</div>
          <button onClick={onClose} style={{
            width: 22, height: 22, borderRadius: 3, padding: 0,
            background: 'transparent', color: hw.textMuted,
            border: `1px solid ${hw.borderDark}`, cursor: 'pointer', fontSize: 13,
          }}>×</button>
        </div>
        <div style={{ fontSize: 12, color: hw.textMuted, marginBottom: 14 }}>
          Add tempo entries at specific positions. Playback BPM follows the map automatically.
        </div>

        {err && (
          <div style={{ fontSize: 11, color: '#e06060', background: 'rgba(224,96,96,0.1)',
            padding: '6px 10px', borderRadius: 4, border: `1px solid rgba(224,96,96,0.3)`, marginBottom: 10 }}>
            {err}
          </div>
        )}

        <div data-testid="tempo-map-list" style={{
          border: `1px solid ${hw.border}`, borderRadius: hw.radius.md,
          background: hw.bgSurface, marginBottom: 14,
        }}>
          <div style={{ display: 'grid', gridTemplateColumns: '40px 1fr 1fr 1fr 40px', ...headerStyle }}>
            <span>#</span>
            <span>Position (beats)</span>
            <span>BPM</span>
            <span>Ramp</span>
            <span></span>
          </div>
          {entries.map((e, i) => (
            <div key={i} data-testid={`tempo-entry-${i}`} style={{
              display: 'grid', gridTemplateColumns: '40px 1fr 1fr 1fr 40px',
              padding: '6px 8px', gap: 8, alignItems: 'center', fontSize: 12,
              borderBottom: i < entries.length - 1 ? `1px solid ${hw.border}` : undefined,
            }}>
              <span style={{ color: hw.textMuted }}>{i}</span>
              <input
                type="number"
                min={0}
                step={0.25}
                value={(e.tick / PPQ).toString()}
                disabled={i === 0}
                onChange={(ev) => handleEdit(i, 'tick', ev.target.value)}
                title={i === 0 ? 'Initial entry is always at position 0' : `Bar ${tickToBar(e.tick, e.timeSigNum)}`}
                style={inputStyle(i === 0)}
              />
              <input
                type="number"
                min={20}
                max={999}
                step={0.1}
                value={e.bpm.toString()}
                onChange={(ev) => handleEdit(i, 'bpm', ev.target.value)}
                style={inputStyle(false)}
              />
              <select
                value={e.ramp}
                onChange={(ev) => handleEdit(i, 'ramp', ev.target.value)}
                style={{ ...inputStyle(false), cursor: 'pointer' }}
              >
                <option value="instant">Instant</option>
                <option value="linear">Linear</option>
              </select>
              {i === 0 ? (
                <span />
              ) : (
                <button
                  data-testid={`tempo-entry-remove-${i}`}
                  onClick={() => handleRemove(i)}
                  title="Remove"
                  style={{
                    width: 24, height: 22, padding: 0,
                    background: 'transparent', color: '#e06060',
                    border: `1px solid ${hw.borderDark}`, borderRadius: 3, cursor: 'pointer', fontSize: 12,
                  }}
                >×</button>
              )}
            </div>
          ))}
        </div>

        <div style={{ display: 'flex', alignItems: 'end', gap: 8, marginBottom: 14 }}>
          <label style={{ display: 'flex', flexDirection: 'column', gap: 3, flex: 1 }}>
            <span style={{ fontSize: 10, color: hw.textMuted }}>Position (beats)</span>
            <input
              data-testid="tempo-new-tick"
              type="number"
              min={0.25}
              step={0.25}
              value={newTickBeats}
              onChange={(e) => setNewTickBeats(e.target.value)}
              style={inputStyle(false)}
            />
          </label>
          <label style={{ display: 'flex', flexDirection: 'column', gap: 3, flex: 1 }}>
            <span style={{ fontSize: 10, color: hw.textMuted }}>BPM</span>
            <input
              data-testid="tempo-new-bpm"
              type="number"
              min={20}
              max={999}
              step={0.1}
              value={newBpm}
              onChange={(e) => setNewBpm(e.target.value)}
              style={inputStyle(false)}
            />
          </label>
          <button
            data-testid="tempo-add"
            onClick={handleAdd}
            style={{
              padding: '7px 14px', fontSize: 12, fontWeight: 600,
              background: 'rgba(124, 58, 237, 0.22)', color: hw.textPrimary,
              border: '1px solid rgba(124, 58, 237, 0.5)', borderRadius: hw.radius.sm,
              cursor: 'pointer',
            }}
          >Add entry</button>
        </div>

        <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
          <button onClick={onClose} style={{
            padding: '8px 16px', fontSize: 13,
            color: hw.textPrimary, background: hw.bgElevated,
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.md, cursor: 'pointer', minWidth: 90,
          }}>Close</button>
        </div>
      </div>
    </div>,
    document.body,
  )
}

function inputStyle(disabled: boolean): React.CSSProperties {
  return {
    width: '100%', fontSize: 12,
    background: disabled ? 'rgba(0,0,0,0.3)' : '#0e0e10',
    color: disabled ? hw.textFaint : hw.textPrimary,
    border: `1px solid ${hw.borderDark}`, borderRadius: 3,
    padding: '4px 6px',
  }
}
