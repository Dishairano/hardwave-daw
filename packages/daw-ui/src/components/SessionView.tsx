import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useNotificationStore } from '../stores/notificationStore'

/**
 * The clip launcher.
 *
 * A timeline is for writing a song down. A grid of loops is for
 * finding one, and for playing live: press a clip and it starts on the
 * next bar, press a row and the row starts together. A cell waiting
 * for the bar is outlined; the one sounding is filled.
 */

interface SlotInfo { name: string; length_ticks?: number; lengthTicks?: number }
interface TrackRow {
  trackId: string
  name: string
  slots: (SlotInfo | null)[]
  playing: number | null
  pending: number
}
interface SessionGrid {
  scenes: string[]
  rows: TrackRow[]
  quantiseBeats: number
}

const QUANTISE = [
  { beats: 0, label: 'now' },
  { beats: 1, label: '1 beat' },
  { beats: 4, label: '1 bar' },
  { beats: 8, label: '2 bars' },
  { beats: 16, label: '4 bars' },
]

export function SessionView({ onClose }: { onClose: () => void }) {
  const [grid, setGrid] = useState<SessionGrid | null>(null)

  const refresh = useCallback(async () => {
    try {
      setGrid(await invoke<SessionGrid>('get_session_grid'))
    } catch { /* no engine in the browser preview */ }
  }, [])

  // The grid shows what the audio thread is doing, so it is read
  // rather than guessed: a launch lands on the bar, not on the click.
  useEffect(() => {
    void refresh()
    const id = setInterval(() => { void refresh() }, 120)
    return () => clearInterval(id)
  }, [refresh])

  const launch = async (trackId: string, slot: number) => {
    try { await invoke('launch_slot', { trackId, slot }) } catch { /* empty slot */ }
    void refresh()
  }

  const fill = async (trackId: string, slot: number) => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const picked = await open({
        multiple: false,
        filters: [{ name: 'Audio', extensions: ['wav', 'flac', 'mp3', 'ogg', 'aac', 'm4a', 'aiff'] }],
      })
      if (typeof picked !== 'string') return
      await invoke('set_session_slot', { trackId, slot, path: picked })
      void refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not load that loop', { detail: String(e) })
    }
  }

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 9700,
        background: 'rgba(0,0,0,0.5)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div style={{
        width: 980, maxWidth: '96vw', maxHeight: '88vh',
        background: hw.bg, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg,
        display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }}>
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 12,
          background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Clip launcher</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>
            a clip starts on the next boundary, a row starts together
          </div>
          <div style={{ flex: 1 }} />
          <span style={{ fontSize: 10, color: hw.textFaint }}>Launch on</span>
          <select
            value={grid?.quantiseBeats ?? 4}
            onChange={e => {
              const beats = Number(e.target.value)
              setGrid(g => g && { ...g, quantiseBeats: beats })
              invoke('set_session_quantise', { beats }).catch(() => {})
            }}
            style={{
              padding: '3px 6px', fontSize: 11, fontFamily: 'inherit',
              background: 'rgba(255,255,255,0.04)', color: hw.textPrimary,
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
            }}
          >
            {QUANTISE.map(q => <option key={q.beats} value={q.beats}>{q.label}</option>)}
          </select>
          <button
            onClick={() => { invoke('stop_all_slots').catch(() => {}); void refresh() }}
            style={btn()}
          >Stop all</button>
          <button onClick={onClose} style={btn()}>Close</button>
        </div>

        <div style={{ overflow: 'auto', padding: 10 }}>
          <table style={{ borderCollapse: 'separate', borderSpacing: 4, width: '100%' }}>
            <thead>
              <tr>
                <th style={{ ...headCell(), textAlign: 'left', width: 120 }}>Scene</th>
                {(grid?.rows ?? []).map(row => (
                  <th key={row.trackId} style={headCell()}>{row.name}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {(grid?.scenes ?? []).map((scene, sceneIndex) => (
                <tr key={scene + sceneIndex}>
                  <td>
                    <button
                      onClick={() => { invoke('launch_scene', { scene: sceneIndex }).catch(() => {}); void refresh() }}
                      style={{ ...btn(), width: '100%', textAlign: 'left' }}
                    >▶ {scene}</button>
                  </td>
                  {(grid?.rows ?? []).map(row => {
                    const slot = row.slots[sceneIndex] ?? null
                    const playing = row.playing === sceneIndex
                    const waiting = row.pending === sceneIndex
                    return (
                      <td key={row.trackId + sceneIndex}>
                        <button
                          onClick={() => slot ? void launch(row.trackId, sceneIndex) : void fill(row.trackId, sceneIndex)}
                          onContextMenu={e => {
                            e.preventDefault()
                            if (!slot) return
                            invoke('clear_session_slot', { trackId: row.trackId, slot: sceneIndex })
                              .catch(() => {})
                              .finally(() => void refresh())
                          }}
                          title={slot ? `${slot.name} — right-click to empty` : 'Empty: click to load a loop'}
                          style={{
                            width: '100%', minWidth: 110, padding: '8px 10px',
                            fontSize: 11, fontFamily: 'inherit', textAlign: 'left',
                            cursor: 'pointer', borderRadius: hw.radius.sm,
                            border: waiting ? `1px dashed ${hw.accent}` : `1px solid ${hw.border}`,
                            background: playing
                              ? hw.accent
                              : slot ? 'rgba(255,255,255,0.07)' : 'rgba(255,255,255,0.02)',
                            color: playing ? '#fff' : slot ? hw.textPrimary : hw.textFaint,
                            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                          }}
                        >
                          {slot ? (playing ? `▶ ${slot.name}` : slot.name) : '+'}
                        </button>
                      </td>
                    )
                  })}
                </tr>
              ))}
              <tr>
                <td>
                  <button
                    onClick={() => { invoke('add_scene', {}).catch(() => {}); void refresh() }}
                    style={{ ...btn(), width: '100%' }}
                  >+ scene</button>
                </td>
                {(grid?.rows ?? []).map(row => (
                  <td key={`stop-${row.trackId}`}>
                    <button
                      onClick={() => { invoke('stop_slot', { trackId: row.trackId }).catch(() => {}); void refresh() }}
                      title={row.pending === -2 ? 'Stopping at the next boundary' : 'Stop at the next boundary'}
                      style={{
                        ...btn(), width: '100%',
                        border: row.pending === -2 ? `1px dashed ${hw.accent}` : 'none',
                      }}
                    >■</button>
                  </td>
                ))}
              </tr>
            </tbody>
          </table>
          {(grid?.rows.length ?? 0) === 0 && (
            <div style={{ padding: 20, fontSize: 11, color: hw.textFaint }}>
              Add a track and its row appears here.
            </div>
          )}
        </div>
      </div>
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

function headCell(): React.CSSProperties {
  return {
    fontSize: 10, fontWeight: 600, color: hw.textFaint,
    textAlign: 'center', padding: '2px 6px', whiteSpace: 'nowrap',
  }
}
