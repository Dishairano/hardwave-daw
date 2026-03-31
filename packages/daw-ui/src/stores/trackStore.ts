import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

interface ClipInfo {
  id: string
  name: string
  kind: string
  position_ticks: number
  length_ticks: number
  muted: boolean
}

interface TrackInfo {
  id: string
  name: string
  kind: string
  color: string
  volume_db: number
  pan: number
  muted: boolean
  soloed: boolean
  armed: boolean
  insert_count: number
}

interface TrackWithClips extends TrackInfo {
  clips: ClipInfo[]
}

interface ImportedClip {
  track_id: string
  clip_id: string
  name: string
  source_id: string
  duration_secs: number
  sample_rate: number
  channels: number
  position_ticks: number
  length_ticks: number
}

interface TrackState {
  tracks: TrackWithClips[]
  selectedTrackId: string | null

  fetchTracks: () => Promise<void>
  selectTrack: (id: string) => void
  addAudioTrack: (name?: string) => Promise<void>
  addMidiTrack: (name?: string) => Promise<void>
  removeTrack: (id: string) => Promise<void>
  setVolume: (id: string, db: number) => Promise<void>
  setPan: (id: string, pan: number) => Promise<void>
  toggleMute: (id: string) => Promise<void>
  toggleSolo: (id: string) => Promise<void>
  importAudioFile: (trackId: string, filePath: string, positionTicks?: number) => Promise<ImportedClip>
}

export const useTrackStore = create<TrackState>((set, get) => ({
  tracks: [],
  selectedTrackId: null,

  fetchTracks: async () => {
    const trackList = await invoke<TrackInfo[]>('get_tracks')
    // Fetch clips for each track
    const tracks: TrackWithClips[] = await Promise.all(
      trackList.map(async (t) => {
        const clips = await invoke<ClipInfo[]>('get_track_clips', { trackId: t.id })
        return { ...t, clips }
      })
    )
    set({ tracks })
  },

  selectTrack: (id) => set({ selectedTrackId: id }),

  addAudioTrack: async (name) => {
    const n = get().tracks.filter(t => t.kind === 'Audio').length + 1
    await invoke('add_audio_track', { name: name || `Audio ${n}` })
    await get().fetchTracks()
  },

  addMidiTrack: async (name) => {
    const n = get().tracks.filter(t => t.kind === 'Midi').length + 1
    await invoke('add_midi_track', { name: name || `MIDI ${n}` })
    await get().fetchTracks()
  },

  removeTrack: async (id) => {
    await invoke('remove_track', { trackId: id })
    await get().fetchTracks()
  },

  setVolume: async (id, db) => {
    await invoke('set_track_volume', { trackId: id, volumeDb: db })
    await get().fetchTracks()
  },

  setPan: async (id, pan) => {
    await invoke('set_track_pan', { trackId: id, pan })
    await get().fetchTracks()
  },

  toggleMute: async (id) => {
    await invoke('toggle_mute', { trackId: id })
    await get().fetchTracks()
  },

  toggleSolo: async (id) => {
    await invoke('toggle_solo', { trackId: id })
    await get().fetchTracks()
  },

  importAudioFile: async (trackId, filePath, positionTicks) => {
    const result = await invoke<ImportedClip>('import_audio_file', {
      trackId,
      filePath,
      positionTicks: positionTicks ?? null,
    })
    await get().fetchTracks()
    return result
  },
}))
