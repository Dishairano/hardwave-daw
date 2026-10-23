/**
 * Markers, the punch range and track folders, saved with the project instead
 * of in browser storage.
 *
 * Both used to live in localStorage, so they belonged to the machine rather
 * than the song: move a .hwp to another computer and every marker was gone,
 * while the next project you opened inherited the markers of the last one.
 * They now ride in the project's `timeline_state` blob, the same ferry
 * `channel_rack_state` uses.
 *
 * Anything already in localStorage is adopted once, on the first project that
 * carries no timeline state, so nobody loses the markers they have today.
 */
import { invoke } from '@tauri-apps/api/core'
import { useMarkerStore, type Marker } from './markerStore'
import { useTransportStore, pushPunchToEngine } from './transportStore'
import { useTrackFolderStore, parseFolders, LEGACY_FOLDERS_KEY, type TrackFolder } from './trackFolderStore'

const LEGACY_MARKERS_KEY = 'hardwave.daw.markers'
const LEGACY_PUNCH_ENABLED = 'hardwave.daw.punchEnabled'
const LEGACY_PUNCH_IN = 'hardwave.daw.punchIn'
const LEGACY_PUNCH_OUT = 'hardwave.daw.punchOut'

export interface PunchState {
  enabled: boolean
  inTicks: number | null
  outTicks: number | null
}

export interface TimelineState {
  markers: Marker[]
  punch: PunchState
  /** Track folders. Absent in songs saved before folders were kept here. */
  folders: TrackFolder[]
}

function ticksOrNull(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
    ? Math.floor(value)
    : null
}

/** Accepts anything; returns only what is structurally sound. */
export function parseTimelineState(json: string | null | undefined): TimelineState | null {
  if (!json) return null
  try {
    const raw = JSON.parse(json) as Partial<TimelineState> | null
    if (!raw || typeof raw !== 'object') return null
    const markers = Array.isArray(raw.markers)
      ? raw.markers.filter((m): m is Marker =>
          !!m && typeof m === 'object'
          && typeof (m as Marker).id === 'string'
          && typeof (m as Marker).tick === 'number'
          && typeof (m as Marker).label === 'string'
          && typeof (m as Marker).color === 'string')
      : []
    const punchRaw = (raw.punch ?? {}) as Partial<PunchState>
    return {
      markers,
      folders: parseFolders((raw as { folders?: unknown }).folders),
      punch: {
        enabled: punchRaw.enabled === true,
        inTicks: ticksOrNull(punchRaw.inTicks),
        outTicks: ticksOrNull(punchRaw.outTicks),
      },
    }
  } catch {
    return null
  }
}

export function currentTimelineState(): TimelineState {
  const t = useTransportStore.getState()
  return {
    markers: useMarkerStore.getState().markers,
    folders: useTrackFolderStore.getState().folders,
    punch: {
      enabled: t.punchEnabled,
      inTicks: t.punchInTicks,
      outTicks: t.punchOutTicks,
    },
  }
}

export function serializeTimelineState(): string {
  return JSON.stringify(currentTimelineState())
}

function applyTimelineState(state: TimelineState) {
  // Loading is not an edit: without this the subscriptions below would push
  // the freshly loaded state straight back at the project.
  suppressPush = true
  try {
    useMarkerStore.getState().setMarkers(state.markers)
    useTrackFolderStore.getState().setFolders(state.folders)
    useTransportStore.setState({
      punchEnabled: state.punch.enabled,
      punchInTicks: state.punch.inTicks,
      punchOutTicks: state.punch.outTicks,
    })
  } finally {
    suppressPush = false
  }
  // The engine keeps its own copy of the punch window, in samples, so a
  // project that opens with a punch range set records inside it without the
  // user touching the controls again.
  pushPunchToEngine()
}

/** Reads the pre-project localStorage keys, then clears them. */
function adoptLegacyTimelineState(): TimelineState | null {
  let markers: Marker[] = []
  let punch: PunchState = { enabled: false, inTicks: null, outTicks: null }
  let found = false
  try {
    const raw = localStorage.getItem(LEGACY_MARKERS_KEY)
    if (raw) {
      const parsed = parseTimelineState(JSON.stringify({ markers: JSON.parse(raw) }))
      if (parsed && parsed.markers.length > 0) {
        markers = parsed.markers
        found = true
      }
    }
    // Number(null) is 0, which would adopt an absent punch point as tick 0.
    const readLegacyTicks = (key: string): number | null => {
      const raw = localStorage.getItem(key)
      if (raw == null || raw === '') return null
      return ticksOrNull(Number(raw))
    }
    const inTicks = readLegacyTicks(LEGACY_PUNCH_IN)
    const outTicks = readLegacyTicks(LEGACY_PUNCH_OUT)
    const enabledRaw = localStorage.getItem(LEGACY_PUNCH_ENABLED)
    if (inTicks != null || outTicks != null || enabledRaw != null) {
      punch = { enabled: enabledRaw === '1' || enabledRaw === 'true', inTicks, outTicks }
      found = true
    }
    for (const key of [LEGACY_MARKERS_KEY, LEGACY_PUNCH_ENABLED, LEGACY_PUNCH_IN, LEGACY_PUNCH_OUT]) {
      localStorage.removeItem(key)
    }
  } catch {
    return null
  }
  return found ? { markers, punch, folders: [] } : null
}

/**
 * Folders from before they were kept with the song, adopted once into the
 * first song opened afterwards: only the ones whose tracks that song has,
 * since track ids repeat from song to song. Then the old copy is removed.
 */
async function adoptLegacyFolders() {
  let raw: string | null = null
  try { raw = localStorage.getItem(LEGACY_FOLDERS_KEY) } catch { return }
  if (!raw) return
  try { localStorage.removeItem(LEGACY_FOLDERS_KEY) } catch { /* still adopt */ }
  let legacy: TrackFolder[] = []
  try { legacy = parseFolders(JSON.parse(raw)) } catch { return }
  if (legacy.length === 0) return
  const ids = new Set(
    (await invoke<{ id: string }[]>('get_tracks').catch(() => [])).map(t => t.id),
  )
  const fitting = legacy
    .map(f => ({ ...f, trackIds: f.trackIds.filter(id => ids.has(id)) }))
    .filter(f => f.trackIds.length > 0)
  if (fitting.length === 0) return
  const have = useTrackFolderStore.getState().folders
  useTrackFolderStore.getState().setFolders([...have, ...fitting.filter(f => !have.some(h => h.id === f.id))])
}

/**
 * Load a project's timeline state into the stores. A project saved before
 * this existed carries none, so anything still in localStorage is adopted
 * that once; otherwise the timeline starts clean, which is what opening a
 * different song should do.
 */
export function hydrateTimelineState(json: string | null) {
  const fromProject = parseTimelineState(json)
  if (fromProject) {
    applyTimelineState(fromProject)
  } else {
    const legacy = adoptLegacyTimelineState()
    applyTimelineState(legacy ?? emptyTimeline())
    if (legacy) void pushTimelineState()
  }
  // The folder store pushes on its own changes, so adopted folders reach
  // the song.
  void adoptLegacyFolders()
}

function emptyTimeline(): TimelineState {
  return { markers: [], punch: { enabled: false, inTicks: null, outTicks: null }, folders: [] }
}

export function resetTimelineState() {
  applyTimelineState(emptyTimeline())
  void pushTimelineState()
}

// Marker edits have to reach the project between explicit saves, or an
// autosave would write a stale timeline. Coalesced so dragging a marker does
// not fire a command per frame.
let pushTimer: ReturnType<typeof setTimeout> | null = null
let suppressPush = false

export function pushTimelineState(): Promise<void> {
  if (pushTimer) clearTimeout(pushTimer)
  return new Promise(resolve => {
    pushTimer = setTimeout(() => {
      pushTimer = null
      invoke('set_timeline_state', { payload: serializeTimelineState() })
        .catch(() => { /* a failed sync must not break a marker edit */ })
        .finally(() => resolve())
    }, 250)
  })
}

/**
 * Keep the project in step with marker and punch edits between saves, so an
 * autosave never writes a timeline one edit behind.
 *
 * The transport store updates on every playhead tick, which is far too often
 * to serialise, so its subscription compares just the punch fields first and
 * returns when nothing that matters changed.
 */
let lastPunchKey = ''
let syncStarted = false

export function startTimelineSync() {
  // React StrictMode runs mount effects twice in development, and the
  // subscriptions are process-wide, so subscribing again would double every
  // push for the life of the window.
  if (syncStarted) return
  syncStarted = true
  useMarkerStore.subscribe(() => {
    if (!suppressPush) void pushTimelineState()
  })
  useTrackFolderStore.subscribe(() => {
    if (!suppressPush) void pushTimelineState()
  })
  useTransportStore.subscribe(s => {
    const key = `${s.punchEnabled}|${s.punchInTicks}|${s.punchOutTicks}`
    if (key === lastPunchKey) return
    lastPunchKey = key
    if (!suppressPush) void pushTimelineState()
  })
}
