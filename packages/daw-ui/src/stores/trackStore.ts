import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { useProjectStore } from './projectStore'
import { useHistoryStore } from './historyStore'

async function mut<T>(cmd: string, args?: Record<string, unknown>, label?: string): Promise<T> {
  const r = await invoke<T>(cmd, args)
  useProjectStore.getState().markDirty()
  if (label) useHistoryStore.getState().push(label)
  return r
}

export type FadeCurveKind = 'linear' | 'equal_power' | 's_curve' | 'logarithmic'

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
  fadeInCurve: FadeCurveKind
  fadeOutCurve: FadeCurveKind
}

export interface InsertInfo {
  id: string
  pluginId: string
  pluginName: string
  enabled: boolean
  wet: number
  sidechainSource: string | null
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
  solo_safe: boolean
  monitorInput: boolean
  phaseInvert: boolean
  swapLr: boolean
  stereoSeparation: number
  delaySamples: number
  pitchSemitones: number
  fineTuneCents: number
  filterType: string
  filterCutoffHz: number
  filterResonance: number
  outputBus: string | null
  insert_count: number
  inserts: InsertInfo[]
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
  addAutomationTrack: (name?: string) => Promise<void>
  removeTrack: (id: string) => Promise<void>
  setVolume: (id: string, db: number) => Promise<void>
  setPan: (id: string, pan: number) => Promise<void>
  toggleMute: (id: string) => Promise<void>
  toggleSolo: (id: string) => Promise<void>
  toggleArm: (id: string) => Promise<void>
  setTrackMonitorInput: (id: string, enabled: boolean) => Promise<void>
  toggleSoloSafe: (id: string) => Promise<void>
  reorderTrack: (id: string, newIndex: number) => Promise<void>
  renameTrack: (id: string, name: string) => Promise<void>
  setTrackColor: (id: string, color: string) => Promise<void>
  setTrackPhaseInvert: (id: string, invert: boolean) => Promise<void>
  setTrackSwapLr: (id: string, swap: boolean) => Promise<void>
  setTrackStereoSeparation: (id: string, separation: number) => Promise<void>
  setTrackDelaySamples: (id: string, samples: number) => Promise<void>
  setTrackPitchSemitones: (id: string, semitones: number) => Promise<void>
  setTrackFineTuneCents: (id: string, cents: number) => Promise<void>
  setTrackFilterType: (id: string, filterType: string) => Promise<void>
  setTrackFilterCutoffHz: (id: string, cutoffHz: number) => Promise<void>
  setTrackFilterResonance: (id: string, resonance: number) => Promise<void>
  setTrackOutputBus: (id: string, outputBus: string | null) => Promise<void>
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
  setClipFadeCurves: (trackId: string, clipId: string, fadeInCurve: FadeCurveKind, fadeOutCurve: FadeCurveKind) => Promise<void>
  toggleClipReverse: (trackId: string, clipId: string) => Promise<void>
  setClipPitch: (trackId: string, clipId: string, pitchSemitones: number) => Promise<void>
  setClipStretch: (trackId: string, clipId: string, stretchRatio: number) => Promise<void>
  autoCrossfadeOverlaps: (trackId?: string) => Promise<number>
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
    const finalName = name || `Audio ${n}`
    await mut('add_audio_track', { name: finalName }, `Add audio track "${finalName}"`)
    await get().fetchTracks()
  },

  addMidiTrack: async (name) => {
    const n = get().tracks.filter(t => t.kind === 'Midi').length + 1
    const finalName = name || `MIDI ${n}`
    await mut('add_midi_track', { name: finalName }, `Add MIDI track "${finalName}"`)
    await get().fetchTracks()
  },

  addAutomationTrack: async (name) => {
    const n = get().tracks.filter(t => t.kind === 'Automation').length + 1
    const finalName = name || `Automation ${n}`
    await mut('add_automation_track', { name: finalName }, `Add automation track "${finalName}"`)
    await get().fetchTracks()
  },

  removeTrack: async (id) => {
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('remove_track', { trackId: id }, `Remove track "${name}"`)
    await get().fetchTracks()
  },

  setVolume: async (id, db) => {
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_volume', { trackId: id, volumeDb: db }, `Set "${name}" volume to ${db.toFixed(1)} dB`)
    await get().fetchTracks()
  },

  setPan: async (id, pan) => {
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_pan', { trackId: id, pan }, `Set "${name}" pan to ${pan.toFixed(2)}`)
    await get().fetchTracks()
  },

  toggleMute: async (id) => {
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('toggle_mute', { trackId: id }, `Toggle mute on "${name}"`)
    await get().fetchTracks()
  },

  toggleSolo: async (id) => {
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('toggle_solo', { trackId: id }, `Toggle solo on "${name}"`)
    await get().fetchTracks()
  },

  toggleArm: async (id) => {
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('toggle_arm', { trackId: id }, `Toggle arm on "${name}"`)
    await get().fetchTracks()
  },

  setTrackMonitorInput: async (id, enabled) => {
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_monitor_input', { trackId: id, enabled }, `${enabled ? 'Enable' : 'Disable'} input monitoring on "${name}"`)
    await get().fetchTracks()
  },

  toggleSoloSafe: async (id) => {
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('toggle_solo_safe', { trackId: id }, `Toggle solo-safe on "${name}"`)
    await get().fetchTracks()
  },

  reorderTrack: async (id, newIndex) => {
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('reorder_track', { trackId: id, newIndex }, `Move "${name}"`)
    await get().fetchTracks()
  },

  renameTrack: async (id, name) => {
    const trimmed = name.trim()
    if (!trimmed) return
    const prev = get().tracks.find(t => t.id === id)?.name ?? ''
    await mut('set_track_name', { trackId: id, name: trimmed }, `Rename "${prev}" to "${trimmed}"`)
    await get().fetchTracks()
  },

  setTrackColor: async (id, color) => {
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_color', { trackId: id, color }, `Recolor "${name}"`)
    await get().fetchTracks()
  },

  setTrackPhaseInvert: async (id, invert) => {
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_phase_invert', { trackId: id, invert }, `${invert ? 'Invert' : 'Un-invert'} phase on "${name}"`)
    await get().fetchTracks()
  },

  setTrackSwapLr: async (id, swap) => {
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_swap_lr', { trackId: id, swap }, `${swap ? 'Swap' : 'Unswap'} L/R on "${name}"`)
    await get().fetchTracks()
  },

  setTrackStereoSeparation: async (id, separation) => {
    const clamped = Math.max(0, Math.min(2, separation))
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_stereo_separation', { trackId: id, separation: clamped }, `Set "${name}" separation to ${clamped.toFixed(2)}`)
    await get().fetchTracks()
  },

  setTrackDelaySamples: async (id, samples) => {
    const clamped = Math.max(0, Math.floor(samples))
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_delay_samples', { trackId: id, samples: clamped }, `Set "${name}" delay to ${clamped} samples`)
    await get().fetchTracks()
  },

  setTrackPitchSemitones: async (id, semitones) => {
    const clamped = Math.max(-24, Math.min(24, Math.round(semitones)))
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_pitch_semitones', { trackId: id, semitones: clamped }, `Set "${name}" pitch to ${clamped > 0 ? '+' : ''}${clamped} st`)
    await get().fetchTracks()
  },

  setTrackFineTuneCents: async (id, cents) => {
    if (!Number.isFinite(cents)) return
    const clamped = Math.max(-100, Math.min(100, cents))
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_fine_tune_cents', { trackId: id, cents: clamped }, `Set "${name}" fine tune to ${clamped > 0 ? '+' : ''}${clamped.toFixed(0)} cents`)
    await get().fetchTracks()
  },

  setTrackFilterType: async (id, filterType) => {
    const normalized = ['off', 'lp', 'hp', 'bp'].includes(filterType) ? filterType : 'off'
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_filter_type', { trackId: id, filterType: normalized }, `Set "${name}" filter to ${normalized}`)
    await get().fetchTracks()
  },

  setTrackFilterCutoffHz: async (id, cutoffHz) => {
    if (!Number.isFinite(cutoffHz)) return
    const clamped = Math.max(20, Math.min(20000, cutoffHz))
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_filter_cutoff', { trackId: id, cutoffHz: clamped }, `Set "${name}" filter cutoff to ${Math.round(clamped)} Hz`)
    await get().fetchTracks()
  },

  setTrackFilterResonance: async (id, resonance) => {
    if (!Number.isFinite(resonance)) return
    const clamped = Math.max(0, Math.min(1, resonance))
    const name = get().tracks.find(t => t.id === id)?.name ?? 'track'
    await mut('set_track_filter_resonance', { trackId: id, resonance: clamped }, `Set "${name}" filter Q to ${clamped.toFixed(2)}`)
    await get().fetchTracks()
  },

  setTrackOutputBus: async (id, outputBus) => {
    const tracks = get().tracks
    const name = tracks.find(t => t.id === id)?.name ?? 'track'
    const targetName = outputBus ? (tracks.find(t => t.id === outputBus)?.name ?? 'Master') : 'Master'
    await mut('set_track_output_bus', { trackId: id, outputBus }, `Route "${name}" to ${targetName}`)
    await get().fetchTracks()
  },

  trackHeights: {},
  setTrackHeight: (id, height) =>
    set(s => ({ trackHeights: { ...s.trackHeights, [id]: Math.max(14, Math.min(240, height)) } })),

  importAudioFile: async (trackId, filePath, positionTicks) => {
    try {
      const name = filePath.split(/[\\/]/).pop() || filePath
      const result = await mut<ImportedClip>('import_audio_file', {
        trackId,
        filePath,
        positionTicks: positionTicks ?? null,
      }, `Import "${name}"`)
      await get().fetchTracks()
      return result
    } catch (err) {
      const { useNotificationStore } = await import('./notificationStore')
      const name = filePath.split(/[\\/]/).pop() || filePath
      useNotificationStore.getState().push('warning',
        `Could not import "${name}"`,
        { detail: String(err), sticky: true },
      )
      throw err
    }
  },

  moveClip: async (trackId, clipId, newPositionTicks) => {
    await mut('move_clip', { trackId, clipId, newPositionTicks }, 'Move clip')
    await get().fetchTracks()
  },

  resizeClip: async (trackId, clipId, newLengthTicks) => {
    await mut('resize_clip', { trackId, clipId, newLengthTicks }, 'Resize clip')
    await get().fetchTracks()
  },

  deleteClip: async (trackId, clipId) => {
    await mut('delete_clip', { trackId, clipId }, 'Delete clip')
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
    const label = toDelete.length === 1 ? 'Delete clip' : `Delete ${toDelete.length} clips`
    for (let i = 0; i < toDelete.length; i++) {
      const { trackId, clipId } = toDelete[i]
      await mut('delete_clip', { trackId, clipId }, i === 0 ? label : undefined)
    }
    set({ selectedClipId: null, selectedClipIds: new Set() })
    await get().fetchTracks()
  },

  duplicateClip: async (trackId, clipId) => {
    const newId = await mut<string>('duplicate_clip', { trackId, clipId }, 'Duplicate clip')
    await get().fetchTracks()
    return newId
  },

  splitClip: async (trackId, clipId, atTicks) => {
    const newId = await mut<string>('split_clip', { trackId, clipId, atTicks }, 'Split clip')
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
    const pasteLabel = clipboard.length === 1 ? 'Paste clip' : `Paste ${clipboard.length} clips`
    for (let i = 0; i < clipboard.length; i++) {
      const c = clipboard[i]
      const offset = c.position_ticks - earliest
      const newPos = positionTicks + offset
      try {
        const newId = await mut<string>('duplicate_clip', { trackId, clipId: c.id }, i === 0 ? pasteLabel : undefined)
        await mut('move_clip', { trackId, clipId: newId, newPositionTicks: newPos })
      } catch (e) {
        console.warn('paste failed for clip', c.id, e)
      }
    }
    await get().fetchTracks()
  },

  setClipGain: async (trackId, clipId, gainDb) => {
    await mut('set_clip_gain', { trackId, clipId, gainDb }, `Set clip gain to ${gainDb.toFixed(1)} dB`)
    await get().fetchTracks()
  },

  setClipFades: async (trackId, clipId, fadeInTicks, fadeOutTicks) => {
    await mut('set_clip_fades', { trackId, clipId, fadeInTicks, fadeOutTicks }, 'Edit clip fades')
    await get().fetchTracks()
  },

  setClipFadeCurves: async (trackId, clipId, fadeInCurve, fadeOutCurve) => {
    await mut('set_clip_fade_curves', { trackId, clipId, fadeInCurve, fadeOutCurve }, 'Edit clip fade curves')
    await get().fetchTracks()
  },

  toggleClipReverse: async (trackId, clipId) => {
    await mut('toggle_clip_reverse', { trackId, clipId }, 'Reverse clip')
    await get().fetchTracks()
  },

  setClipPitch: async (trackId, clipId, pitchSemitones) => {
    await mut('set_clip_pitch', { trackId, clipId, pitchSemitones }, `Pitch clip ${pitchSemitones >= 0 ? '+' : ''}${pitchSemitones} st`)
    await get().fetchTracks()
  },

  setClipStretch: async (trackId, clipId, stretchRatio) => {
    await mut('set_clip_stretch', { trackId, clipId, stretchRatio }, `Set clip stretch ${stretchRatio.toFixed(2)}×`)
    await get().fetchTracks()
  },

  autoCrossfadeOverlaps: async (trackId) => {
    const { tracks } = get()
    const targets = trackId ? tracks.filter(t => t.id === trackId) : tracks
    let pairs = 0
    let firstLabel = true
    for (const track of targets) {
      if (track.kind === 'Master') continue
      const clips = [...track.clips].sort((a, b) => a.position_ticks - b.position_ticks)
      for (let j = 1; j < clips.length; j++) {
        const prev = clips[j - 1]
        const cur = clips[j]
        const prevEnd = prev.position_ticks + prev.length_ticks
        if (cur.position_ticks < prevEnd) {
          const overlap = prevEnd - cur.position_ticks
          const fadeOut = Math.min(overlap, prev.length_ticks)
          const fadeIn = Math.min(overlap, cur.length_ticks)
          await mut(
            'set_clip_fades',
            { trackId: track.id, clipId: prev.id, fadeInTicks: prev.fadeInTicks, fadeOutTicks: fadeOut },
            firstLabel ? 'Auto-crossfade overlaps' : undefined,
          )
          firstLabel = false
          await mut(
            'set_clip_fades',
            { trackId: track.id, clipId: cur.id, fadeInTicks: fadeIn, fadeOutTicks: cur.fadeOutTicks },
          )
          pairs++
        }
      }
    }
    if (pairs > 0) await get().fetchTracks()
    return pairs
  },

  undo: async () => {
    const ok = await invoke<boolean>('undo')
    if (ok) {
      useProjectStore.getState().markDirty()
      useHistoryStore.getState().undoOne()
      await get().fetchTracks()
    }
    return ok
  },

  redo: async () => {
    const ok = await invoke<boolean>('redo')
    if (ok) {
      useProjectStore.getState().markDirty()
      useHistoryStore.getState().redoOne()
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
