import { invoke } from '@tauri-apps/api/core'
import { useTrackStore, type TrackInfo } from './trackStore'
import { usePluginStore } from './pluginStore'
import { t } from '../i18n'

/**
 * Instrument channels: a MIDI track in the channel rack with something
 * that plays on it. The channel rack's "+ Instrument" makes them, and so
 * does the empty piano roll's "Add an instrument".
 */

export const SAMPLER_ID = 'hardwave.native.sampler'
export const TRIPLE_OSC_ID = 'hardwave.native.tripleosc'

/** A new channel with this instrument plug-in on it. Returns its id. With
 *  `select`, the channel is picked, as the rack does after adding one. */
export async function addInstrumentChannel(pluginId: string, name: string, select = true): Promise<string> {
  const trackId = await invoke<string>('add_midi_track', { name })
  await usePluginStore.getState().addToTrack(trackId, pluginId)
  const ts = useTrackStore.getState()
  await ts.fetchTracks()
  if (select) ts.selectTrack(trackId)
  return trackId
}

/** A new channel playing the built-in kick synth. Returns its id. */
export async function addKickSynthChannel(select = true): Promise<string> {
  const trackId = await invoke<string>('add_midi_track', { name: 'Kick' })
  await invoke('set_track_instrument', { trackId, kind: 'kick_synth' })
  const ts = useTrackStore.getState()
  await ts.fetchTracks()
  if (select) ts.selectTrack(trackId)
  return trackId
}

function fileName(path: string): string {
  return path.split(/[\\/]/).pop() || path
}

function stem(name: string): string {
  const dot = name.lastIndexOf('.')
  return dot > 0 ? name.slice(0, dot) : name
}

/** The sample a sampler channel plays, as a full path; null for any other channel. */
export function channelSamplePath(track: TrackInfo): string | null {
  return track.inserts?.find(s => s.pluginId === SAMPLER_ID)?.samplePath ?? null
}

/**
 * What a channel plays, for the small grey label beside its name: "Kick
 * synth", or the sample's name ("Clap 1", or "Clap 1.wav" with
 * `withExtension`). Null when there is nothing to add to the name.
 */
export function channelSound(track: TrackInfo, withExtension = false): string | null {
  const sample = channelSamplePath(track)
  if (sample) return withExtension ? fileName(sample) : stem(fileName(sample))
  const hasPlugin = (track.inserts ?? []).length > 0
  if (track.instrument === 'kick_synth' && !hasPlugin) return t('Kick synth')
  return null
}

/**
 * A new song's channels: Kick (the kick synth), Clap, Hat and Snare, each
 * with its sound loaded, the way FL opens. The engine leaves a song whose
 * first rows are in use alone, and a missing drums folder leaves the song
 * as it was; either way the song opens.
 */
export async function startNewSong(): Promise<void> {
  try {
    await invoke('add_new_song_channels')
  } catch (err) {
    console.warn('The new song opened without its drums:', err)
  }
  await useTrackStore.getState().fetchTracks()
}
