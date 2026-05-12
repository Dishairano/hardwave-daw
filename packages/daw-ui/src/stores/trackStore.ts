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

const TRACK_LIMIT = 500

/**
 * Compute the next default track name. Counts every non-Master track
 * (Audio + MIDI + Automation + Bus + Return) and returns "Track N+1",
 * mirroring FL Studio's unified channel numbering. Returns null if the
 * 500-track cap has been reached.
 */
function nextTrackName(tracks: { kind: string }[]): string | null {
  const used = tracks.filter(t => t.kind !== 'Master').length
  if (used >= TRACK_LIMIT) return null
  return `Track ${used + 1}`
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
  automationLanes: AutomationLaneInfo[]
  /** Native voicing for MIDI tracks. `'sine'` is the default
   *  monosynth, `'kick_synth'` swaps in Hardwave's 4-layer kick. */
  instrument?: NativeInstrumentId
  /** Per-track KickSynth layer patch. 4 layers, each may be null
   *  meaning "use engine default for this layer". Only meaningful
   *  when `instrument === 'kick_synth'`. */
  kickPatch?: { layers: (KickLayerPatch | null)[]; drive: number }
}

export interface KickLayerPatch {
  peak_gain: number
  length_secs: number
  release_secs: number
  sweep_start_hz: number
  sweep_end_hz: number
  sweep_secs: number
  waveform: 'sine' | 'saw' | 'square' | 'triangle'
}

export type NativeInstrumentId = 'builtin_sine' | 'kick_synth'

export type AutomationTargetInfo =
  | { kind: 'track_volume' }
  | { kind: 'track_pan' }
  | { kind: 'track_mute' }
  | { kind: 'plugin_param'; slotId: string; paramId: number }
  | { kind: 'send_level'; sendIndex: number }

export interface AutomationPointInfo {
  tick: number
  value: number
  curve: string
  tension: number
}

export interface AutomationLaneInfo {
  id: string
  target: AutomationTargetInfo
  points: AutomationPointInfo[]
  visible: boolean
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
  /// Per-id lookup map — same data as `tracks`, indexed by id for O(1)
  /// access. Maintained automatically by `fetchTracks` and every
  /// optimistic mutator that touches `tracks`. Used by the fine-grained
  /// `useTrackVolume` / `useTrackPan` / etc. selectors so each strip's
  /// per-field subscription is a hash lookup instead of an O(N) `find`.
  tracksById: Record<string, TrackWithClips>
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
  addAudioTrack: (name?: string) => Promise<string | null>
  addMidiTrack: (name?: string) => Promise<void>
  addAutomationTrack: (name?: string) => Promise<void>
  removeTrack: (id: string) => Promise<void>
  setVolume: (id: string, db: number) => Promise<void>
  setPan: (id: string, pan: number) => Promise<void>
  /// Optimistic local-only volume update — mutates the track's volume in
  /// the store without an IPC round-trip or fetchTracks. Used by the
  /// fader's pointermove path so 60fps drag isn't bottlenecked by a
  /// full-store refetch. Caller MUST call commitVolume (or setVolume) on
  /// pointerup to persist the final value to the backend.
  setVolumeLocal: (id: string, db: number) => void
  /// Same idea as setVolumeLocal but for pan.
  setPanLocal: (id: string, pan: number) => void
  /// Persist a track's final post-drag volume to the backend in one IPC.
  /// Triggers fetchTracks once at the end. Call this on pointerup after
  /// a drag that used setVolumeLocal.
  commitVolume: (id: string, db: number) => Promise<void>
  /// Same idea as commitVolume but for pan.
  commitPan: (id: string, pan: number) => Promise<void>
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

  // Native instrument selection (MIDI tracks only)
  setTrackInstrument: (trackId: string, kind: NativeInstrumentId) => Promise<void>

  // Automation
  addAutomationLane: (trackId: string, target: AutomationTargetInfo) => Promise<string>
  deleteAutomationLane: (trackId: string, laneId: string) => Promise<void>
  addAutomationPoint: (trackId: string, laneId: string, tick: number, value: number) => Promise<number>
  moveAutomationPoint: (trackId: string, laneId: string, pointIndex: number, tick: number, value: number) => Promise<number>
  deleteAutomationPoint: (trackId: string, laneId: string, pointIndex: number) => Promise<void>
  setAutomationLaneVisible: (trackId: string, laneId: string, visible: boolean) => Promise<void>
  importAudioFile: (trackId: string, filePath: string, positionTicks?: number) => Promise<ImportedClip>
  moveClip: (trackId: string, clipId: string, newPositionTicks: number) => Promise<void>
  /// Optimistic local-only update — mutates the clip's position in the
  /// store without an IPC round-trip or fetchTracks. Used during a
  /// drag's mousemove path so 60fps drag isn't bottlenecked by 500-track
  /// state refresh. Caller MUST call moveClip (or commitClipDrag) on
  /// mouseup to persist the final position to the backend.
  moveClipLocal: (trackId: string, clipId: string, newPositionTicks: number) => void
  /// Same idea as moveClipLocal but for resize — local-only length update.
  resizeClipLocal: (trackId: string, clipId: string, newLengthTicks: number) => void
  /// Persist a clip's final post-drag position+length to the backend in
  /// one IPC + one fetchTracks. Call this once on mouseup after a drag
  /// that used moveClipLocal/resizeClipLocal.
  commitClipDrag: (
    trackId: string,
    clipId: string,
    newPositionTicks: number,
    newLengthTicks?: number,
  ) => Promise<void>
  moveClipToTrack: (
    fromTrackId: string,
    toTrackId: string,
    clipId: string,
    newPositionTicks: number,
  ) => Promise<void>
  resizeClip: (trackId: string, clipId: string, newLengthTicks: number) => Promise<void>
  deleteClip: (trackId: string, clipId: string) => Promise<void>
  deleteSelectedClip: () => Promise<void>
  deleteSelectedClips: () => Promise<void>
  duplicateClip: (trackId: string, clipId: string) => Promise<string>
  /// Picker place-mode: clone an existing clip and drop it on (possibly
  /// a different) track at the given tick. One commit, one fetch — the
  /// canvas calls this on left-click while the picker has an audio
  /// clip selected.
  placeClipCopy: (
    sourceTrackId: string,
    sourceClipId: string,
    targetTrackId: string,
    positionTicks: number,
  ) => Promise<string>
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

/// Build a fresh `tracksById` from a `tracks` array. Keep this in one
/// place so `fetchTracks` and the optimistic mutators (`setVolumeLocal`,
/// `setPanLocal`) agree on the index shape.
function indexTracks(tracks: TrackWithClips[]): Record<string, TrackWithClips> {
  const idx: Record<string, TrackWithClips> = {}
  for (const t of tracks) idx[t.id] = t
  return idx
}

export const useTrackStore = create<TrackState>((set, get) => ({
  tracks: [],
  tracksById: {},
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
    // One IPC round-trip via get_tracks_with_clips replaces the old
    // pattern of get_tracks + N × get_track_clips (1 + 500 = 501 calls
    // for a default-sized session). See docs/perf-audit.md hotspot #5.
    try {
      const payload = await invoke<Array<TrackInfo & { clips: ClipInfo[] }>>(
        'get_tracks_with_clips',
      )
      const tracks: TrackWithClips[] = payload.map((p) => ({ ...p, clips: p.clips ?? [] }))
      set({ tracks, tracksById: indexTracks(tracks) })
    } catch (e) {
      // Fallback for older backends that haven't shipped the one-shot
      // command yet (e.g. hot-swap frontend update against a stale
      // Rust binary).
      console.warn('get_tracks_with_clips unavailable; falling back to legacy multi-call path', e)
      const trackList = await invoke<TrackInfo[]>('get_tracks')
      const tracks: TrackWithClips[] = await Promise.all(
        trackList.map(async (t) => {
          const clips = await invoke<ClipInfo[]>('get_track_clips', { trackId: t.id })
          return { ...t, clips }
        }),
      )
      set({ tracks, tracksById: indexTracks(tracks) })
    }
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
    const finalName = name || nextTrackName(get().tracks)
    if (!finalName) {
      console.warn('Track limit reached (500)')
      return null
    }
    // Backend returns the new track id; surface it so callers (e.g. the
    // drag-drop importer) can target the freshly-created track without
    // racing against fetchTracks() repopulating the store.
    const id = await mut<string>('add_audio_track', { name: finalName }, `Add audio track "${finalName}"`)
    await get().fetchTracks()
    return id
  },

  addMidiTrack: async (name) => {
    const finalName = name || nextTrackName(get().tracks)
    if (!finalName) {
      console.warn('Track limit reached (500)')
      return
    }
    await mut('add_midi_track', { name: finalName }, `Add MIDI track "${finalName}"`)
    await get().fetchTracks()
  },

  addAutomationTrack: async (name) => {
    const finalName = name || nextTrackName(get().tracks)
    if (!finalName) {
      console.warn('Track limit reached (500)')
      return
    }
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

  // ---- optimistic-local versions for high-frequency drag/wheel updates ----
  // These mutate the store in place without IPC or fetchTracks. Pair every
  // drag session with a single commitVolume/commitPan on pointerup so the
  // backend + redo history catch up. Mirrors moveClipLocal/commitClipDrag.
  setVolumeLocal: (id, db) => {
    set((s) => {
      const tracks = s.tracks.map((t) => (t.id === id ? { ...t, volume_db: db } : t))
      const tracksById = s.tracksById[id]
        ? { ...s.tracksById, [id]: { ...s.tracksById[id], volume_db: db } }
        : s.tracksById
      return { tracks, tracksById }
    })
  },
  setPanLocal: (id, pan) => {
    set((s) => {
      const tracks = s.tracks.map((t) => (t.id === id ? { ...t, pan } : t))
      const tracksById = s.tracksById[id]
        ? { ...s.tracksById, [id]: { ...s.tracksById[id], pan } }
        : s.tracksById
      return { tracks, tracksById }
    })
  },
  commitVolume: async (id, db) => {
    const name = get().tracks.find((t) => t.id === id)?.name ?? 'track'
    await mut(
      'set_track_volume',
      { trackId: id, volumeDb: db },
      `Set "${name}" volume to ${db.toFixed(1)} dB`,
    )
    await get().fetchTracks()
  },
  commitPan: async (id, pan) => {
    const name = get().tracks.find((t) => t.id === id)?.name ?? 'track'
    await mut(
      'set_track_pan',
      { trackId: id, pan },
      `Set "${name}" pan to ${pan.toFixed(2)}`,
    )
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

  setTrackInstrument: async (trackId, kind) => {
    await invoke('set_track_instrument', { trackId, kind })
    await get().fetchTracks()
  },

  // ─── Automation ─────────────────────────────────────────────────
  // Each call mutates the project on the Rust side and triggers an
  // engine rebuild; we then refetch tracks to pick up the new lane
  // / point shape. Mutations are rare (one click per gesture), so the
  // round-trip is fine — drag-move calls are coalesced UI-side
  // before they hit invoke().
  addAutomationLane: async (trackId, target) => {
    const id = await invoke<string>('add_automation_lane', { trackId, target })
    await get().fetchTracks()
    return id
  },
  deleteAutomationLane: async (trackId, laneId) => {
    await invoke('delete_automation_lane', { trackId, laneId })
    await get().fetchTracks()
  },
  addAutomationPoint: async (trackId, laneId, tick, value) => {
    const idx = await invoke<number>('add_automation_point', { trackId, laneId, tick, value })
    await get().fetchTracks()
    return idx
  },
  moveAutomationPoint: async (trackId, laneId, pointIndex, tick, value) => {
    const idx = await invoke<number>('move_automation_point', { trackId, laneId, pointIndex, tick, value })
    await get().fetchTracks()
    return idx
  },
  deleteAutomationPoint: async (trackId, laneId, pointIndex) => {
    await invoke('delete_automation_point', { trackId, laneId, pointIndex })
    await get().fetchTracks()
  },
  setAutomationLaneVisible: async (trackId, laneId, visible) => {
    await invoke('set_automation_lane_visible', { trackId, laneId, visible })
    await get().fetchTracks()
  },

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

  moveClipLocal: (trackId, clipId, newPositionTicks) => {
    set(state => ({
      tracks: state.tracks.map(t => {
        if (t.id !== trackId || !t.clips) return t
        return {
          ...t,
          clips: t.clips.map(c =>
            c.id === clipId ? { ...c, position_ticks: newPositionTicks } : c,
          ),
        }
      }),
    }))
  },

  resizeClipLocal: (trackId, clipId, newLengthTicks) => {
    set(state => ({
      tracks: state.tracks.map(t => {
        if (t.id !== trackId || !t.clips) return t
        return {
          ...t,
          clips: t.clips.map(c =>
            c.id === clipId ? { ...c, length_ticks: newLengthTicks } : c,
          ),
        }
      }),
    }))
  },

  commitClipDrag: async (trackId, clipId, newPositionTicks, newLengthTicks) => {
    // Single IPC roundtrip at the end of the drag. We deliberately skip
    // the per-frame fetchTracks() that the regular moveClip/resizeClip
    // would do — the store has been kept in sync optimistically by
    // moveClipLocal/resizeClipLocal during the drag, so one final
    // fetchTracks at the end is enough to reconcile with the backend.
    await invoke('move_clip', { trackId, clipId, newPositionTicks })
    if (newLengthTicks !== undefined) {
      await invoke('resize_clip', { trackId, clipId, newLengthTicks })
    }
    await get().fetchTracks()
  },

  moveClipToTrack: async (fromTrackId, toTrackId, clipId, newPositionTicks) => {
    await mut(
      'move_clip_to_track',
      { fromTrackId, toTrackId, clipId, newPositionTicks },
      'Move clip to track',
    )
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

  placeClipCopy: async (sourceTrackId, sourceClipId, targetTrackId, positionTicks) => {
    const newId = await mut<string>(
      'duplicate_clip',
      { trackId: sourceTrackId, clipId: sourceClipId },
      'Place clip',
    )
    if (sourceTrackId === targetTrackId) {
      await invoke('move_clip', { trackId: targetTrackId, clipId: newId, newPositionTicks: positionTicks })
    } else {
      await invoke('move_clip_to_track', {
        fromTrackId: sourceTrackId,
        toTrackId: targetTrackId,
        clipId: newId,
        newPositionTicks: positionTicks,
      })
    }
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

// ---- fine-grained selector hooks ----
// Each selector reads through `tracksById` for O(1) lookup. Without this
// every selector was `s.tracks.find(t => t.id === id)` — O(N) per
// selector × 8 selectors per strip × 500 strips = 2 million comparisons
// per store update. See docs/perf-audit.md hotspot #4.
export const useTrackById = (id: string) =>
  useTrackStore((s) => s.tracksById[id])
export const useTrackVolume = (id: string) =>
  useTrackStore((s) => s.tracksById[id]?.volume_db ?? 0)
export const useTrackPan = (id: string) =>
  useTrackStore((s) => s.tracksById[id]?.pan ?? 0)
export const useTrackStereoSeparation = (id: string) =>
  useTrackStore((s) => s.tracksById[id]?.stereoSeparation ?? 1)
export const useTrackMuted = (id: string) =>
  useTrackStore((s) => s.tracksById[id]?.muted ?? false)
export const useTrackSoloed = (id: string) =>
  useTrackStore((s) => s.tracksById[id]?.soloed ?? false)
export const useTrackArmed = (id: string) =>
  useTrackStore((s) => s.tracksById[id]?.armed ?? false)
export const useTrackName = (id: string) =>
  useTrackStore((s) => s.tracksById[id]?.name ?? '')
export const useTrackKind = (id: string) =>
  useTrackStore((s) => s.tracksById[id]?.kind)
export const useTrackColor = (id: string) =>
  useTrackStore((s) => s.tracksById[id]?.color)
/// Track ids in render order — stable across content updates, only changes
/// when tracks are added/removed/reordered.
export const useTrackIds = () =>
  useTrackStore((s) => s.tracks.map((t) => t.id))
/// Subset of track ids by kind (audio / midi / automation / bus / return /
/// master). Useful for splitting the mixer's insert column from buses.
export const useTrackIdsByKind = (kind: string) =>
  useTrackStore((s) => s.tracks.filter((t) => t.kind === kind).map((t) => t.id))
