import { useTrackStore } from './trackStore'

/**
 * Give the piano roll something to draw in: with no clip open in it and an
 * instrument channel picked, that channel gets a clip (its first, or a new
 * one) and the roll opens it.
 */
export async function bindPianoRollToSelection(): Promise<void> {
  const ts = useTrackStore.getState()
  if (ts.activeMidiClipId) return
  const trackId = ts.selectedTrackId
  if (!trackId) return
  const track = ts.tracks.find(t => t.id === trackId)
  if (!track || track.kind !== 'Midi') return
  const clipId = await ts.ensureMidiClipOnTrack(trackId)
  if (clipId && !useTrackStore.getState().activeMidiClipId) ts.setActiveMidiClip(trackId, clipId)
}

/**
 * While the piano roll is open: bind it now, and again each time another
 * channel is picked. It only bound when it opened, so a new producer who
 * opened the piano roll first and then added an instrument saw "No clip
 * selected", and clicking in it did nothing. Returns the unsubscribe.
 */
export function followSelectionInPianoRoll(): () => void {
  void bindPianoRollToSelection().catch(() => {})
  let last = useTrackStore.getState().selectedTrackId
  return useTrackStore.subscribe(s => {
    if (s.selectedTrackId === last) return
    last = s.selectedTrackId
    void bindPianoRollToSelection().catch(() => {})
  })
}
