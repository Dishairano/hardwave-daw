import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

export interface ClipInfo {
  id: string
  name: string
  kind: string
  source_id: string
  position_ticks: number
  length_ticks: number
  muted: boolean
}

export interface TrackInfo {
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

export interface TrackWithClips extends TrackInfo {
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

// Cache waveform peaks per source_id
const waveformCache = new Map<string, [number, number][]>()

interface TrackState {
  tracks: TrackWithClips[]
  selectedTrackId: string | null
  selectedClipId: string | null

  fetchTracks: () => Promise<void>
  selectTrack: (id: string) => void
  selectClip: (clipId: string | null, trackId?: string) => void
  addAudioTrack: (name?: string) => Promise<void>
  addMidiTrack: (name?: string) => Promise<void>
  removeTrack: (id: string) => Promise<void>
  setVolume: (id: string, db: number) => Promise<void>
  setPan: (id: string, pan: number) => Promise<void>
  toggleMute: (id: string) => Promise<void>
  toggleSolo: (id: string) => Promise<void>
  toggleArm: (id: string) => Promise<void>
  reorderTrack: (id: string, newIndex: number) => Promise<void>
  trackHeights: Record<string, number>
  setTrackHeight: (id: string, height: number) => void
  importAudioFile: (trackId: string, filePath: string, positionTicks?: number) => Promise<ImportedClip>
  moveClip: (trackId: string, clipId: string, newPositionTicks: number) => Promise<void>
  resizeClip: (trackId: string, clipId: string, newLengthTicks: number) => Promise<void>
  deleteClip: (trackId: string, clipId: string) => Promise<void>
  deleteSelectedClip: () => Promise<void>
  getWaveformPeaks: (sourceId: string, numBuckets: number) => Promise<[number, number][]>
}

export const useTrackStore = create<TrackState>((set, get) => ({
  tracks: [],
  selectedTrackId: null,
  selectedClipId: null,

  fetchTracks: async () => {
    const trackList = await invoke<TrackInfo[]>('get_tracks')
    const tracks: TrackWithClips[] = await Promise.all(
      trackList.map(async (t) => {
        const clips = await invoke<ClipInfo[]>('get_track_clips', { trackId: t.id })
        return { ...t, clips }
      })
    )
    set({ tracks })
  },

  selectTrack: (id) => set({ selectedTrackId: id }),

  selectClip: (clipId, trackId) => set({
    selectedClipId: clipId,
    ...(trackId ? { selectedTrackId: trackId } : {}),
  }),

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

  toggleArm: async (id) => {
    await invoke('toggle_arm', { trackId: id })
    await get().fetchTracks()
  },

  reorderTrack: async (id, newIndex) => {
    await invoke('reorder_track', { trackId: id, newIndex })
    await get().fetchTracks()
  },

  trackHeights: {},
  setTrackHeight: (id, height) =>
    set(s => ({ trackHeights: { ...s.trackHeights, [id]: Math.max(24, Math.min(240, height)) } })),

  importAudioFile: async (trackId, filePath, positionTicks) => {
    const result = await invoke<ImportedClip>('import_audio_file', {
      trackId,
      filePath,
      positionTicks: positionTicks ?? null,
    })
    await get().fetchTracks()
    return result
  },

  moveClip: async (trackId, clipId, newPositionTicks) => {
    await invoke('move_clip', { trackId, clipId, newPositionTicks })
    await get().fetchTracks()
  },

  resizeClip: async (trackId, clipId, newLengthTicks) => {
    await invoke('resize_clip', { trackId, clipId, newLengthTicks })
    await get().fetchTracks()
  },

  deleteClip: async (trackId, clipId) => {
    await invoke('delete_clip', { trackId, clipId })
    set({ selectedClipId: null })
    await get().fetchTracks()
  },

  deleteSelectedClip: async () => {
    const { selectedClipId, tracks } = get()
    if (!selectedClipId) return
    for (const track of tracks) {
      const clip = track.clips.find(c => c.id === selectedClipId)
      if (clip) {
        await get().deleteClip(track.id, clip.id)
        return
      }
    }
  },

  getWaveformPeaks: async (sourceId, numBuckets) => {
    const cacheKey = `${sourceId}:${numBuckets}`
    if (waveformCache.has(cacheKey)) {
      return waveformCache.get(cacheKey)!
    }
    const peaks = await invoke<[number, number][]>('get_waveform_peaks', { sourceId, numBuckets })
    waveformCache.set(cacheKey, peaks)
    return peaks
  },
}))
