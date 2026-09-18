/**
 * What the browser's right-click menu does with an audio file, modelled on
 * FL Studio's browser menu: send the sample to the selected channel, open it
 * in a new sampler channel, drop it on the playlist at the playhead, show it
 * in Explorer, or move it to the recycle bin.
 *
 * A "channel" here is a MIDI track holding the Hardwave Sampler, the same
 * thing the Channel Rack's instrument picker creates.
 */
import { invoke } from '@tauri-apps/api/core'
import { useTrackStore, type TrackInfo } from '../../stores/trackStore'
import { useBrowserStore } from '../../stores/browserStore'

export const SAMPLER_ID = 'hardwave.native.sampler'

export function fileStem(path: string): string {
  const name = path.split(/[\\/]/).pop() || path
  const dot = name.lastIndexOf('.')
  return dot > 0 ? name.slice(0, dot) : name
}

/** The playhead in ticks, through the tempo map (0 when there is no backend). */
async function playheadTick(): Promise<number> {
  try { return await invoke<number>('get_playhead_tick') } catch { return 0 }
}

function samplerSlot(track: TrackInfo | undefined) {
  return track?.inserts?.find(s => s.pluginId === SAMPLER_ID)
}

/**
 * What "Send to selected channel" would do for the selected track, or why it
 * cannot. A MIDI track with another instrument on it is left alone: loading a
 * sample must never replace someone's synth.
 */
export type SendTarget =
  | { kind: 'sampler'; trackId: string; slotId: string; name: string }
  | { kind: 'emptyMidi'; trackId: string; name: string }
  | { kind: 'audio'; trackId: string; name: string }
  | { kind: 'none'; reason: string }

export function selectedSendTarget(): SendTarget {
  const { tracks, selectedTrackId } = useTrackStore.getState()
  const track = tracks.find(t => t.id === selectedTrackId)
  if (!track || track.kind === 'Master') return { kind: 'none', reason: 'No channel selected' }
  if (track.kind === 'Audio') return { kind: 'audio', trackId: track.id, name: track.name }
  if (track.kind === 'Midi') {
    const slot = samplerSlot(track)
    if (slot) return { kind: 'sampler', trackId: track.id, slotId: slot.id, name: track.name }
    if (!track.inserts?.length) return { kind: 'emptyMidi', trackId: track.id, name: track.name }
    return { kind: 'none', reason: `${track.name} holds another instrument` }
  }
  return { kind: 'none', reason: 'Select an audio or instrument channel' }
}

async function loadIntoSampler(trackId: string, slotId: string, path: string) {
  await invoke('load_sampler', { trackId, slotId, path })
}

export async function sendToSelectedChannel(path: string) {
  const target = selectedSendTarget()
  const store = useTrackStore.getState()
  switch (target.kind) {
    case 'sampler':
      await loadIntoSampler(target.trackId, target.slotId, path)
      break
    case 'emptyMidi': {
      const slotId = await invoke<string>('add_plugin_to_track', { trackId: target.trackId, pluginId: SAMPLER_ID })
      await loadIntoSampler(target.trackId, slotId, path)
      await store.fetchTracks()
      break
    }
    case 'audio':
      await store.importAudioFile(target.trackId, path, await playheadTick())
      break
    case 'none':
      return
  }
  useBrowserStore.getState().pushFileRecent(path)
}

/** A new channel holding the Hardwave Sampler with this file loaded. */
export async function openInNewChannel(path: string) {
  const trackId = await invoke<string>('add_midi_track', { name: fileStem(path) })
  const slotId = await invoke<string>('add_plugin_to_track', { trackId, pluginId: SAMPLER_ID })
  await loadIntoSampler(trackId, slotId, path)
  const store = useTrackStore.getState()
  await store.fetchTracks()
  store.selectTrack(trackId)
  useBrowserStore.getState().pushFileRecent(path)
}

/**
 * An audio clip at the playhead on the selected audio track, or the first
 * audio track, or a new one when the project has none.
 */
export async function sendToPlaylistAsClip(path: string) {
  const store = useTrackStore.getState()
  const audio = store.tracks.filter(t => t.kind === 'Audio')
  let trackId: string | null =
    audio.find(t => t.id === store.selectedTrackId)?.id ?? audio[0]?.id ?? null
  if (!trackId) trackId = await store.addAudioTrack()
  if (!trackId) return
  await useTrackStore.getState().importAudioFile(trackId, path, await playheadTick())
  useBrowserStore.getState().pushFileRecent(path)
}

/** A new audio track named after the file, with the clip at the playhead. */
export async function sendToPlaylistAsAudioTrack(path: string) {
  const trackId = await useTrackStore.getState().addAudioTrack(fileStem(path))
  if (!trackId) return
  await useTrackStore.getState().importAudioFile(trackId, path, await playheadTick())
  useTrackStore.getState().selectTrack(trackId)
  useBrowserStore.getState().pushFileRecent(path)
}

export async function showInFolder(path: string) {
  const { revealItemInDir } = await import('@tauri-apps/plugin-opener')
  await revealItemInDir(path)
}

/**
 * Move the file to the recycle bin after asking, then forget it everywhere
 * the browser remembers it. Returns false when the user said no.
 */
export async function trashFile(path: string): Promise<boolean> {
  const name = path.split(/[\\/]/).pop() || path
  let ok = false
  try {
    const { ask } = await import('@tauri-apps/plugin-dialog')
    ok = await ask(`Move "${name}" to the recycle bin?`, { title: 'Delete file', kind: 'warning', okLabel: 'Delete', cancelLabel: 'Cancel' })
  } catch {
    ok = window.confirm(`Move "${name}" to the recycle bin?`)
  }
  if (!ok) return false
  await invoke('trash_browser_file', { path })
  const b = useBrowserStore.getState()
  if (b.fileFavorites.has(path)) b.toggleFileFavorite(path)
  b.removeFileRecent(path)
  b.clearFileTags(path)
  window.dispatchEvent(new CustomEvent('daw:browserFileRemoved', { detail: path }))
  return true
}
