import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'
import { useNotificationStore } from '../../stores/notificationStore'

/**
 * Which other instruments a MIDI track's notes also play.
 *
 * Layering a supersaw under a lead meant copying the clip onto the second
 * track and keeping the two copies in step by hand. A route leaves the
 * notes in one place: the ticked tracks play the same clips through their
 * own instrument and their own chain.
 */
export function MidiRouteDialog({ trackId, onClose }: { trackId: string; onClose: () => void }) {
  const tracks = useTrackStore(s => s.tracks)
  const track = tracks.find(t => t.id === trackId)
  const candidates = tracks.filter(t => t.kind === 'Midi' && t.id !== trackId)
  const [chosen, setChosen] = useState<string[]>([])
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    invoke<string[]>('get_midi_routes', { trackId })
      .then(list => { setChosen(list); setLoaded(true) })
      .catch(() => setLoaded(true))
  }, [trackId])

  const toggle = useCallback((id: string) => {
    setChosen(prev => (prev.includes(id) ? prev.filter(x => x !== id) : [...prev, id]))
  }, [])

  const save = useCallback(async () => {
    try {
      await invoke('set_midi_routes', { trackId, targets: chosen })
      await useTrackStore.getState().fetchTracks()
      useNotificationStore.getState().push('info', chosen.length === 0
        ? 'Notes go to this track only'
        : `Notes also play ${chosen.length} other ${chosen.length === 1 ? 'track' : 'tracks'}`)
      onClose()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not set that route', { detail: String(e) })
    }
  }, [trackId, chosen, onClose])

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 10001,
        background: 'rgba(0,0,0,0.45)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div style={{
        width: 380, maxWidth: '92vw', maxHeight: '70vh',
        background: hw.bg, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg,
        overflow: 'hidden', display: 'flex', flexDirection: 'column',
      }}>
        <div style={{
          padding: '8px 12px', background: hw.bgElevated,
          borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>
            Send the notes of "{track?.name ?? 'this track'}" on
          </div>
          <div style={{ fontSize: 9, color: hw.textFaint, marginTop: 2 }}>
            Each ticked track plays the same notes through its own instrument.
          </div>
        </div>

        <div style={{ overflowY: 'auto', flex: 1, padding: 6 }}>
          {candidates.length === 0 && (
            <div style={{ padding: 10, fontSize: 10, color: hw.textFaint }}>
              There is no other MIDI track to send them to yet.
            </div>
          )}
          {candidates.map(t => (
            <label
              key={t.id}
              style={{
                display: 'flex', alignItems: 'center', gap: 8,
                padding: '5px 8px', fontSize: 11, cursor: 'pointer',
              }}
            >
              <input
                type="checkbox"
                checked={chosen.includes(t.id)}
                onChange={() => toggle(t.id)}
                style={{ accentColor: hw.accent }}
              />
              <span>{t.name}</span>
            </label>
          ))}
        </div>

        <div style={{
          padding: '8px 12px', display: 'flex', gap: 8, justifyContent: 'flex-end',
          background: hw.bgElevated, borderTop: `1px solid ${hw.border}`,
        }}>
          <button onClick={onClose} style={btn()}>Cancel</button>
          <button onClick={() => { void save() }} disabled={!loaded} style={btn(true)}>Save</button>
        </div>
      </div>
    </div>
  )
}

function btn(active: boolean = false) {
  return {
    padding: '3px 12px', fontSize: 10, background: 'transparent',
    border: `1px solid ${active ? hw.accent : hw.border}`, borderRadius: hw.radius.sm,
    color: active ? hw.accent : hw.textSecondary, cursor: 'pointer',
  } as const
}
