import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'
import { useNotificationStore } from '../../stores/notificationStore'
import { DialogFrame } from '../ui/DialogFrame'

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
    <DialogFrame
      title={`Send the notes of "${track?.name ?? 'this track'}" on`}
      subtitle="Each ticked track plays the same notes through its own instrument."
      onClose={onClose}
      width={420}
      footer={<>
        <button type="button" className="hw-dbtn" onClick={onClose}>Cancel</button>
        <button type="button" className="hw-dbtn primary" onClick={() => { void save() }} disabled={!loaded}>Save</button>
      </>}
    >
      <div style={{ overflowY: 'auto', flex: 1, padding: 6 }}>
        {candidates.length === 0 && (
          <div style={{ padding: 10, fontSize: 12, color: hw.textFaint }}>
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

    </DialogFrame>
  )
}

