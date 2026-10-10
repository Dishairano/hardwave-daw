import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useTrackStore, type TrackInfo } from '../stores/trackStore'
import { DialogFrame } from './ui/DialogFrame'
import { btn, sel, th, td } from './ui/dialogStyles'

interface MidiMappingsPanelProps {
  onClose: () => void
  initialTarget?: MidiMapTarget
}

export type MidiMapTarget =
  | { kind: 'masterVolume' }
  | { kind: 'trackVolume'; trackId: string }
  | { kind: 'trackPan'; trackId: string }
  | { kind: 'trackMute'; trackId: string }
  | { kind: 'pluginParam'; trackId: string; slotId: string; paramId: number }

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

function targetLabel(t: MidiMapTarget, tracks: TrackInfo[]): string {
  if (t.kind === 'masterVolume') return 'Master volume'
  const tr = tracks.find(x => x.id === t.trackId)
  const name = tr ? tr.name : `track:${t.trackId.slice(0, 6)}`
  if (t.kind === 'trackVolume') return `${name} — Volume`
  if (t.kind === 'trackPan') return `${name} — Pan`
  if (t.kind === 'trackMute') return `${name} — Mute`
  return `${name} — Plug-in ${t.slotId.slice(0, 4)} param ${t.paramId}`
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
  // `initialTarget?.kind` includes `'pluginParam'`, which isn't a valid
  // TargetKind for the Add-mapping dropdown — that dropdown only adds
  // generic track/master mappings. Plugin-param mappings come from the
  // Channel Rack's MIDI Learn flow and are filtered to a safe default.
  const [addKind, setAddKind] = useState<TargetKind>(
    initialTarget && initialTarget.kind !== 'pluginParam'
      ? initialTarget.kind
      : 'masterVolume'
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
    <DialogFrame
      title="MIDI Mappings"
      subtitle="CC → parameter · persisted to user config"
      onClose={onClose}
      width={560}
      headerActions={<>
          {mappings.length > 0 && (
            <button onClick={clearAll} style={btn()}>Clear all</button>
          )}
      </>}
    >
      <div style={{
        padding: 12, background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        display: 'flex', gap: 10, alignItems: 'center', flexWrap: 'wrap',
      }}>
        <span style={{ fontSize: 12, color: hw.textSecondary }}>Add mapping:</span>
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
          padding: '8px 12px', fontSize: 12, color: hw.accent,
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
          <div style={{ fontSize: 12.5, color: hw.textFaint, textAlign: 'center', padding: 24 }}>
            No mappings yet. Pick a target above and click "Start learn".
          </div>
        ) : (
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12.5 }}>
            <thead>
              <tr style={{ color: hw.textFaint, fontSize: 11, textTransform: 'uppercase' }}>
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
        background: hw.bgElevated, fontSize: 11, color: hw.textFaint,
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
    </DialogFrame>
  )
}

