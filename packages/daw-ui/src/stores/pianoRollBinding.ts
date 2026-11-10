import { invoke } from '@tauri-apps/api/core'
import { useTrackStore } from './trackStore'
import { useProjectStore } from './projectStore'
import { addInstrumentChannel, addKickSynthChannel, TRIPLE_OSC_ID } from './instrumentChannels'

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

/**
 * Open the piano roll on a channel's notes: its first clip, or a new one,
 * the same way picking the channel with the roll open does. The channel is
 * picked after the clip is open, so following the selection finds the roll
 * busy and makes no second clip.
 */
export async function openChannelInPianoRoll(trackId: string): Promise<void> {
  const clipId = await useTrackStore.getState().ensureMidiClipOnTrack(trackId)
  if (!clipId) return
  useTrackStore.getState().setActiveMidiClip(trackId, clipId)
  useTrackStore.getState().selectTrack(trackId)
}

/** An instrument for a new channel: a plug-in, or the kick synth. */
export type NewInstrument = { pluginId: string; name: string } | 'kick_synth'

async function addChannel(instrument: NewInstrument): Promise<string> {
  return instrument === 'kick_synth'
    ? addKickSynthChannel(false)
    : addInstrumentChannel(instrument.pluginId, instrument.name, false)
}

/** The empty piano roll's "Add an instrument": a new channel in the rack,
 *  with a clip, open in the roll. Returns the channel's id. */
export async function addInstrumentToPianoRoll(instrument: NewInstrument): Promise<string> {
  const trackId = await addChannel(instrument)
  await openChannelInPianoRoll(trackId)
  return trackId
}

/**
 * A click in the empty piano roll is never ignored: it adds a Hardwave
 * 3xOSC channel and puts the note where the click was. The instrument can
 * be swapped later; a click that did nothing taught a new producer nothing.
 */
export async function addSynthWithNote(note: {
  pitch: number
  startTick: number
  durationTicks: number
  velocity: number
}): Promise<string | null> {
  const trackId = await addChannel({ pluginId: TRIPLE_OSC_ID, name: 'Hardwave 3xOSC' })
  const clipId = await useTrackStore.getState().ensureMidiClipOnTrack(trackId)
  if (!clipId) return null
  await invoke('add_midi_note', { trackId, clipId, ...note })
  useProjectStore.getState().markDirty()
  // Opened after the note is in, so the roll's first look at the clip has it.
  useTrackStore.getState().setActiveMidiClip(trackId, clipId)
  useTrackStore.getState().selectTrack(trackId)
  return trackId
}
