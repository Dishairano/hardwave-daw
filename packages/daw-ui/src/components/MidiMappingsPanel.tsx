import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useTrackStore, type TrackInfo } from '../stores/trackStore'

interface MidiMappingsPanelProps {
  onClose: () => void
  initialTarget?: MidiMapTarget
}

export type MidiMapTarget =
  | { kind: 'masterVolume' }
  | { kind: 'trackVolume'; trackId: string }
  | { kind: 'trackPan'; trackId: string }
  | { kind: 'trackMute'; trackId: string }

interface MidiMapping {
  id: number
  cc: number
  channel: number | null
  target: MidiMapTarget
}

interface LearnStatus {
  learning: boolean
  target: MidiMapTarget | null
  lastLearned: MidiMapping | null
}

type TargetKind = 'masterVolume' | 'trackVolume' | 'trackPan' | 'trackMute'

function targetKey(t: MidiMapTarget): string {
  if (t.kind === 'masterVolume') return 'master-volume'
  return `${t.kind}-${t.trackId}`
}

function targetLabel(t: MidiMapTarget, tracks: TrackInfo[]): string {
  if (t.kind === 'masterVolume') return 'Master volume'
  const tr = tracks.find(x => x.id === t.trackId)
  const name = tr ? tr.name : `track:${t.trackId.slice(0, 6)}`
  if (t.kind === 'trackVolume') return `${name} — Volume`
  if (t.kind === 'trackPan') return `${name} — Pan`
  return `${name} — Mute`
}

function buildTarget(kind: TargetKind, trackId: string): MidiMapTarget {
  if (kind === 'masterVolume') return { kind: 'masterVolume' }
  return { kind, trackId }
}

export function MidiMappingsPanel({ onClose, initialTarget }: MidiMappingsPanelProps) {
  const tracks = useTrackStore(s => s.tracks)
  const [mappings, setMappings] = useState<MidiMapping[]>([])
  const [learnStatus, setLearnStatus] = useState<LearnStatus>({
    learning: false, target: null, lastLearned: null,
  })
  const [addKind, setAddKind] = useState<TargetKind>(
    initialTarget?.kind ?? 'masterVolume'
  )
  const [addTrackId, setAddTrackId] = useState<string>(
    initialTarget && initialTarget.kind !== 'masterVolume'
      ? initialTarget.trackId
      : (tracks[0]?.id ?? '')
  )

  const refreshMappings = useCallback(async () => {
    try {
      const list = await invoke<MidiMapping[]>('list_midi_mappings')
      setMappings(list)
    } catch { /* ignore */ }
  }, [])

  const refreshStatus = useCallback(async () => {
    try {
      const s = await invoke<LearnStatus>('midi_learn_status')
      setLearnStatus(s)
      if (s.lastLearned) {
        await refreshMappings()
      }
    } catch { /* ignore */ }
  }, [refreshMappings])

  useEffect(() => { refreshMappings() }, [refreshMappings])

  useEffect(() => {
    const id = setInterval(refreshStatus, 200)
    return () => clearInterval(id)
  }, [refreshStatus])

  useEffect(() => {
    if (!addTrackId && tracks.length > 0) setAddTrackId(tracks[0].id)
  }, [tracks, addTrackId])

  const startLearn = useCallback(async (preset?: MidiMapTarget) => {
    const target = preset ?? buildTarget(addKind, addTrackId)
    if (target.kind !== 'masterVolume' && !target.trackId) return
    try {
      await invoke('midi_learn_start', { target })
      await refreshStatus()
    } catch (e) {
      console.error('midi_learn_start failed', e)
    }
  }, [addKind, addTrackId, refreshStatus])

  useEffect(() => {
    if (!initialTarget) return
    startLearn(initialTarget)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const cancelLearn = async () => {
    await invoke('midi_learn_cancel')
    await refreshStatus()
  }

  const removeMapping = async (id: number) => {
    await invoke('remove_midi_mapping', { id })
    await refreshMappings()
  }

  const clearAll = async () => {
    await invoke('clear_midi_mappings')
    await refreshMappings()
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
      <div
        style={{
          width: 560, maxWidth: '94vw', maxHeight: '80vh',
          background: hw.bg, color: hw.textPrimary,
          border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg,
          overflow: 'hidden', display: 'flex', flexDirection: 'column',
        }}
      >
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center',
          gap: 12, background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>MIDI Mappings</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>
            CC → parameter · persisted to user config
          </div>
          <div style={{ flex: 1 }} />
          {mappings.length > 0 && (
            <button onClick={clearAll} style={btn()}>Clear all</button>
          )}
          <button onClick={onClose} style={btn()}>Close</button>
        </div>

        <div style={{
          padding: 12, background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
          display: 'flex', gap: 10, alignItems: 'center', flexWrap: 'wrap',
        }}>
          <span style={{ fontSize: 10, color: hw.textSecondary }}>Add mapping:</span>
          <select
            value={addKind}
            onChange={(e) => setAddKind(e.target.value as TargetKind)}
            disabled={learnStatus.learning}
            style={sel()}
          >
            <option value="masterVolume">Master volume</option>
            <option value="trackVolume">Track volume</option>
            <option value="trackPan">Track pan</option>
            <option value="trackMute">Track mute</option>
          </select>
          {addKind !== 'masterVolume' && (
            <select
              value={addTrackId}
              onChange={(e) => setAddTrackId(e.target.value)}
              disabled={learnStatus.learning}
              style={sel()}
            >
              {tracks.length === 0 && <option value="">(no tracks)</option>}
              {tracks.map(t => (
                <option key={t.id} value={t.id}>{t.name}</option>
              ))}
            </select>
          )}
          {learnStatus.learning
            ? <button onClick={cancelLearn} style={btn(true)}>Cancel learn</button>
            : <button onClick={() => startLearn()} style={btn(false)}>Start learn</button>
          }
        </div>

        {learnStatus.learning && (
          <div style={{
            padding: '8px 12px', fontSize: 10, color: hw.accent,
            background: hw.bg, borderBottom: `1px solid ${hw.border}`,
          }}>
            Move a MIDI controller now. The first CC received will be mapped to&nbsp;
            <strong>
              {learnStatus.target ? targetLabel(learnStatus.target, tracks) : ''}
            </strong>.
          </div>
        )}

        <div style={{
          flex: 1, overflow: 'auto', padding: 8,
        }}>
          {mappings.length === 0 ? (
            <div style={{ fontSize: 11, color: hw.textFaint, textAlign: 'center', padding: 24 }}>
              No mappings yet. Pick a target above and click "Start learn".
            </div>
          ) : (
            <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 11 }}>
              <thead>
                <tr style={{ color: hw.textFaint, fontSize: 9, textTransform: 'uppercase' }}>
                  <th style={th()}>CC</th>
                  <th style={th()}>Ch</th>
                  <th style={th()}>Target</th>
                  <th style={th()}></th>
                </tr>
              </thead>
              <tbody>
                {mappings.map(m => (
                  <tr key={m.id} style={{ borderTop: `1px solid ${hw.border}` }}>
                    <td style={td()}>{m.cc}</td>
                    <td style={td()}>{m.channel === null ? 'any' : m.channel + 1}</td>
                    <td style={td()}>{targetLabel(m.target, tracks)}</td>
                    <td style={{ ...td(), textAlign: 'right' }}>
                      <button onClick={() => removeMapping(m.id)} style={btn()}>Remove</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>

        <div style={{
          padding: '6px 12px', borderTop: `1px solid ${hw.border}`,
          background: hw.bgElevated, fontSize: 9, color: hw.textFaint,
        }}>
          <strong>Tip:</strong> open a MIDI input device in Audio Settings
          before trying to learn. Value range: volume 0→-60..0 dB, pan 0→-1..+1,
          mute &gt; 0.5 = muted.
          {learnStatus.lastLearned && (
            <span style={{ color: hw.accent, marginLeft: 8 }}>
              · Last captured: CC {learnStatus.lastLearned.cc} · {' '}
              {targetLabel(learnStatus.lastLearned.target, tracks)}
            </span>
          )}
          {mappings.some(m =>
            m.target.kind !== 'masterVolume'
            && !tracks.some(t => t.id === (m.target as { trackId: string }).trackId)
          ) && (
            <span style={{ color: hw.red, marginLeft: 8 }}>
              · Warning: some mappings reference missing tracks.
            </span>
          )}
        </div>
      </div>
    </div>
  )
}

function btn(active: boolean = false) {
  return {
    padding: '3px 10px', fontSize: 10, background: 'transparent',
    border: `1px solid ${active ? hw.accent : hw.border}`, borderRadius: hw.radius.sm,
    color: active ? hw.accent : hw.textSecondary, cursor: 'pointer',
    transition: 'color 0.15s, border-color 0.15s',
  } as const
}

function sel() {
  return {
    fontSize: 10, padding: '2px 6px', background: hw.bg,
    border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
    color: hw.textPrimary,
  } as const
}

function th() {
  return { textAlign: 'left' as const, padding: '6px 8px', fontWeight: 600 }
}

function td() {
  return { padding: '6px 8px', fontVariantNumeric: 'tabular-nums' as const }
}
