import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { useProjectStore } from './projectStore'

async function mut<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const r = await invoke<T>(cmd, args)
  useProjectStore.getState().markDirty()
  return r
}

export interface ClipInfo {
  id: string
  name: string
  kind: string
  source_id: string
  position_ticks: number
  length_ticks: number
  muted: boolean
  gainDb: number
  fadeInTicks: number
  fadeOutTicks: number
  reversed: boolean
  pitchSemitones: number
  stretchRatio: number
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
  selectedClipIds: Set<string>
  clipboard: ClipInfo[]
  activeMidiTrackId: string | null
  activeMidiClipId: string | null

  fetchTracks: () => Promise<void>
  selectTrack: (id: string) => void
  selectClip: (clipId: string | null, trackId?: string) => void
  setActiveMidiClip: (trackId: string | null, clipId: string | null) => void
  ensureMidiClipOnTrack: (trackId: string) => Promise<string | null>
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
  deleteSelectedClips: () => Promise<void>
  duplicateClip: (trackId: string, clipId: string) => Promise<string>
  splitClip: (trackId: string, clipId: string, atTicks: number) => Promise<string>
  toggleClipSelection: (clipId: string) => void
  clearSelection: () => void
  selectAllClips: () => void
  copySelectedClips: () => void
  pasteClipsAtPosition: (positionTicks: number, targetTrackId?: string) => Promise<void>
  setClipGain: (trackId: string, clipId: string, gainDb: number) => Promise<void>
  setClipFades: (trackId: string, clipId: string, fadeInTicks: number, fadeOutTicks: number) => Promise<void>
  toggleClipReverse: (trackId: string, clipId: string) => Promise<void>
  setClipPitch: (trackId: string, clipId: string, pitchSemitones: number) => Promise<void>
  setClipStretch: (trackId: string, clipId: string, stretchRatio: number) => Promise<void>
  undo: () => Promise<boolean>
  redo: () => Promise<boolean>
  getWaveformPeaks: (sourceId: string, numBuckets: number) => Promise<[number, number][]>
}

export const useTrackStore = create<TrackState>((set, get) => ({
  tracks: [],
  selectedTrackId: null,
  selectedClipId: null,
  selectedClipIds: new Set(),
  clipboard: [],
  activeMidiTrackId: null,
  activeMidiClipId: null,

  setActiveMidiClip: (trackId, clipId) => set({ activeMidiTrackId: trackId, activeMidiClipId: clipId }),

  ensureMidiClipOnTrack: async (trackId) => {
    const state = get()
    const track = state.tracks.find(t => t.id === trackId)
    if (!track || track.kind !== 'Midi') return null
    const existing = track.clips.find(c => c.kind === 'midi')
    if (existing) return existing.id
    const newClipId = await mut<string>('create_midi_clip', {
      trackId,
      name: 'MIDI Clip',
      positionTicks: 0,
      lengthTicks: null,
    })
    await get().fetchTracks()
    return newClipId
  },

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
    selectedClipIds: clipId ? new Set([clipId]) : new Set(),
    ...(trackId ? { selectedTrackId: trackId } : {}),
  }),

  toggleClipSelection: (clipId) => set(s => {
    const next = new Set(s.selectedClipIds)
    if (next.has(clipId)) next.delete(clipId); else next.add(clipId)
    const primary = next.has(s.selectedClipId || '') ? s.selectedClipId : (next.values().next().value ?? null)
    return { selectedClipIds: next, selectedClipId: primary as string | null }
  }),

  clearSelection: () => set({ selectedClipId: null, selectedClipIds: new Set() }),

  selectAllClips: () => set(s => {
    const ids = new Set<string>()
    for (const t of s.tracks) for (const c of t.clips) ids.add(c.id)
    const primary = ids.values().next().value ?? null
    return { selectedClipIds: ids, selectedClipId: primary as string | null }
  }),

  addAudioTrack: async (name) => {
    const n = get().tracks.filter(t => t.kind === 'Audio').length + 1
    await mut('add_audio_track', { name: name || `Audio ${n}` })
    await get().fetchTracks()
  },

  addMidiTrack: async (name) => {
    const n = get().tracks.filter(t => t.kind === 'Midi').length + 1
    await mut('add_midi_track', { name: name || `MIDI ${n}` })
    await get().fetchTracks()
  },

  removeTrack: async (id) => {
    await mut('remove_track', { trackId: id })
    await get().fetchTracks()
  },

  setVolume: async (id, db) => {
    await mut('set_track_volume', { trackId: id, volumeDb: db })
    await get().fetchTracks()
  },

  setPan: async (id, pan) => {
    await mut('set_track_pan', { trackId: id, pan })
    await get().fetchTracks()
  },

  toggleMute: async (id) => {
    await mut('toggle_mute', { trackId: id })
    await get().fetchTracks()
  },

  toggleSolo: async (id) => {
    await mut('toggle_solo', { trackId: id })
    await get().fetchTracks()
  },

  toggleArm: async (id) => {
    await mut('toggle_arm', { trackId: id })
    await get().fetchTracks()
  },

  reorderTrack: async (id, newIndex) => {
    await mut('reorder_track', { trackId: id, newIndex })
    await get().fetchTracks()
  },

  trackHeights: {},
  setTrackHeight: (id, height) =>
    set(s => ({ trackHeights: { ...s.trackHeights, [id]: Math.max(24, Math.min(240, height)) } })),

  importAudioFile: async (trackId, filePath, positionTicks) => {
    const result = await mut<ImportedClip>('import_audio_file', {
      trackId,
      filePath,
      positionTicks: positionTicks ?? null,
    })
    await get().fetchTracks()
    return result
  },

  moveClip: async (trackId, clipId, newPositionTicks) => {
    await mut('move_clip', { trackId, clipId, newPositionTicks })
    await get().fetchTracks()
  },

  resizeClip: async (trackId, clipId, newLengthTicks) => {
    await mut('resize_clip', { trackId, clipId, newLengthTicks })
    await get().fetchTracks()
  },

  deleteClip: async (trackId, clipId) => {
    await mut('delete_clip', { trackId, clipId })
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

  deleteSelectedClips: async () => {
    const ids = Array.from(get().selectedClipIds)
    if (ids.length === 0) { await get().deleteSelectedClip(); return }
    const { tracks } = get()
    const toDelete: Array<{ trackId: string; clipId: string }> = []
    for (const track of tracks) {
      for (const clip of track.clips) {
        if (ids.includes(clip.id)) toDelete.push({ trackId: track.id, clipId: clip.id })
      }
    }
    for (const { trackId, clipId } of toDelete) {
      await mut('delete_clip', { trackId, clipId })
    }
    set({ selectedClipId: null, selectedClipIds: new Set() })
    await get().fetchTracks()
  },

  duplicateClip: async (trackId, clipId) => {
    const newId = await mut<string>('duplicate_clip', { trackId, clipId })
    await get().fetchTracks()
    return newId
  },

  splitClip: async (trackId, clipId, atTicks) => {
    const newId = await mut<string>('split_clip', { trackId, clipId, atTicks })
    await get().fetchTracks()
    return newId
  },

  copySelectedClips: () => {
    const { selectedClipIds, tracks } = get()
    const ids = Array.from(selectedClipIds)
    const collected: ClipInfo[] = []
    for (const t of tracks) for (const c of t.clips) if (ids.includes(c.id)) collected.push(c)
    set({ clipboard: collected })
  },

  pasteClipsAtPosition: async (positionTicks, targetTrackId) => {
    const { clipboard, tracks, selectedTrackId } = get()
    if (clipboard.length === 0) return
    const trackId = targetTrackId || selectedTrackId
    if (!trackId) return
    const track = tracks.find(t => t.id === trackId)
    if (!track) return
    // Normalize: earliest clip aligns to positionTicks; others preserve relative offset.
    const earliest = clipboard.reduce((m, c) => Math.min(m, c.position_ticks), Infinity)
    for (const c of clipboard) {
      const offset = c.position_ticks - earliest
      const newPos = positionTicks + offset
      try {
        const newId = await mut<string>('duplicate_clip', { trackId, clipId: c.id })
        await mut('move_clip', { trackId, clipId: newId, newPositionTicks: newPos })
      } catch (e) {
        console.warn('paste failed for clip', c.id, e)
      }
    }
    await get().fetchTracks()
  },

  setClipGain: async (trackId, clipId, gainDb) => {
    await mut('set_clip_gain', { trackId, clipId, gainDb })
    await get().fetchTracks()
  },

  setClipFades: async (trackId, clipId, fadeInTicks, fadeOutTicks) => {
    await mut('set_clip_fades', { trackId, clipId, fadeInTicks, fadeOutTicks })
    await get().fetchTracks()
  },

  toggleClipReverse: async (trackId, clipId) => {
    await mut('toggle_clip_reverse', { trackId, clipId })
    await get().fetchTracks()
  },

  setClipPitch: async (trackId, clipId, pitchSemitones) => {
    await mut('set_clip_pitch', { trackId, clipId, pitchSemitones })
    await get().fetchTracks()
  },

  setClipStretch: async (trackId, clipId, stretchRatio) => {
    await mut('set_clip_stretch', { trackId, clipId, stretchRatio })
    await get().fetchTracks()
  },

  undo: async () => {
    const ok = await invoke<boolean>('undo')
    if (ok) {
      useProjectStore.getState().markDirty()
      await get().fetchTracks()
    }
    return ok
  },

  redo: async () => {
    const ok = await invoke<boolean>('redo')
    if (ok) {
      useProjectStore.getState().markDirty()
      await get().fetchTracks()
    }
    return ok
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
