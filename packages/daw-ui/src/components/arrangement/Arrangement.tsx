import { magnetEdge, magnetSnap } from './magnetSnap'
import { labelGap, rulerLabel } from './rulerLabel'
import { followScroll } from './follow'
import { usePlaylistScrollStore } from '../../stores/playlistScrollStore'
import { useDropTarget } from '../../lib/pointerDrag'
import {
  rowIndexOf, itemAt, itemsInBox, itemsInView, rowAtY, shownRowCount, silentRows, trackPlaces,
  type RowItem,
} from './rowIndex'
import { usePlaylistRowsStore } from '../../stores/playlistRowsStore'
import { useRef, useEffect, useState, useCallback, useMemo } from 'react'
import { TuneDialog } from './TuneDialog'
import { StemsDialog } from './StemsDialog'
import { useCollabStore } from '../../stores/collabStore'
import { SpectralEditor } from './SpectralEditor'
import { listen } from '@tauri-apps/api/event'
import { useTrackStore, ClipInfo, ClipDragEdit, FadeCurveKind, WavePeak, type AutomationClipInfo, type AutomationTargetInfo, type TrackWithClips } from '../../stores/trackStore'
import { useSampleWindowStore } from '../../stores/sampleWindowStore'
import { invoke } from '@tauri-apps/api/core'
import { sectionAt, sectionMoves } from './sectionMoves'
import { useTransportStore, snapToTicks } from '../../stores/transportStore'
import { useShallow } from 'zustand/react/shallow'
import { useMarkerStore } from '../../stores/markerStore'
import { useClipGroupStore } from '../../stores/clipGroupStore'
import { useNotificationStore } from '../../stores/notificationStore'
import { useHistoryStore } from '../../stores/historyStore'
import { usePickerStore } from '../../stores/pickerStore'
import { usePlaylistToolStore } from '../../stores/playlistToolStore'
import { rippleMoveShifts } from '../../stores/ripple'
import { useGeneralPrefsStore } from '../../stores/generalPrefsStore'
import { useColorPickerStore } from '../../stores/colorPickerStore'
import { useLogStore } from '../../dev/logStore'
import { useMultiTouchGestures } from '../../hooks/useMultiTouchGestures'
import { hw } from '../../theme'
import { paintSlotOccupied, paintSlotTick } from './paint'
import { zoomStep } from './zoom'
import { useTempoMapStore } from '../../stores/tempoMapStore'
import { useProjectStore } from '../../stores/projectStore'
import { reloadTimelineState } from '../../stores/timelineState'
import { bpmAt, flagEntry, tempoFlags } from './tempoFlags'
import { layoutBpm, sampleAtTick, tickAtSample } from '../../utils/tempoTime'
import { fileLayout, stretchLabel } from './clipTime'
import { TEMPO_LANE_HEIGHT, drawTempoLane, laneScale } from './tempoLaneDraw'
import { TempoLane } from './TempoLane'
import { gridLines } from '../../utils/meter'
import { mixHex, hexLuma } from '../../lib/color'
import { rgbColour } from '../../lib/waveformColour'
import { t, tf, useLanguage } from '../../i18n'

const PPQ = 960
const PIXELS_PER_SECOND_BASE = 100
// (mockup-aligned)
const RESIZE_HANDLE_PX = 6
// Canvas-internal bar ruler — 22 px reserved at the top for bar numbers
// drawn in the same pixel-space as the grid lines underneath. The HTML
// ruler in HwApp.tsx is hidden via .fl-pl-ruler-hidden when the canvas
// owns the ruler so we never have two competing visual layers.
const BAR_RULER_HEIGHT = 22
// Where the track rows start: under the ruler, and under the tempo lane when
// it is shown. Set as the arrangement renders (there is one), so every row,
// hit test and scroll reads the same top without each carrying the lane.
let RULER_HEIGHT = BAR_RULER_HEIGHT

type DragMode =
  | 'none'
  | 'move'
  | 'resize-right'
  | 'resize-left'
  | 'fade-in'
  | 'fade-out'
  | 'rubber'
  | 'scrub'
  // Drag inside a clip with the slip tool: slides the audio within the clip
  // while the clip keeps its position and length on the timeline.
  | 'slip'
  // Drag across the playlist with the paint tool: drops a copy of the armed
  // clip in every grid slot the pointer crosses.
  | 'paint'
  | 'pending-empty'
  // Ctrl/⌘ + drag in the ruler band → define a loop region. The drag's
  // origin tick anchors one edge, the live cursor tick anchors the
  // other; mouseup commits via transport.setLoop and enables looping.
  | 'loop-range'

// Pixel distance the pointer has to move from mousedown before an
// empty-area press promotes into a rubber-band selection. Below this
// threshold a left-click is treated as a no-op (or as a paste if the
// picker has something to drop) — matching the FL Studio model where a
// pure click is "paint", and only a held drag is "select".
const RUBBER_PROMOTE_THRESHOLD_PX = 4

interface DragState {
  mode: DragMode
  clipId: string
  trackId: string
  startMouseX: number
  startMouseY: number
  currentMouseX: number
  currentMouseY: number
  originalPositionTicks: number
  originalLengthTicks: number
  originalFadeInTicks: number
  originalFadeOutTicks: number
  groupMoveOriginals?: { clipId: string; trackId: string; origPos: number; origRow: number }[]
  /** The playlist row the dragged clip was on when the drag started, and
   *  the row the pointer started on: a drag up or down moves the clips
   *  that many rows, on their own channels. */
  originalRow?: number
  startRow?: number
  /** How many rows the drag has moved the clips so far. */
  lastRowDelta?: number
  /** An automation clip is being dragged (its id is `clipId`); a point of
   *  it when `pointIndex` is set. */
  automation?: { pointIndex?: number; pointTick?: number; pointValue?: number }
  /** The other clips' edges and the markers, for magnetic snap. Gathered on
   *  the first move of a drag and kept for the rest of it. */
  snapTargets?: number[]
  // Scroll offset frozen at scrub start. The auto-follow scroll offset is
  // derived from the live playhead, so recomputing it mid-scrub while
  // playing makes a stationary cursor map to an ever-advancing tick (the
  // playhead chases itself). Freezing it keeps screen-X → tick stable.
  scrubScrollOffset?: number
  // Paint drag only. `paintedSlots` holds `trackId:tick` for every slot this
  // gesture has already filled, so dragging back and forth over one slot
  // cannot stack copies. `paintChain` serialises the placements: each one is
  // two IPC calls, and firing them in parallel makes the backend interleave
  // a duplicate with another slot's move. Slip, fade and clip drags send
  // through it too, so their release waits for what is still on the way.
  paintedSlots?: Set<string>
  paintChain?: Promise<void>
  paintStepTicks?: number
  // Slip drag only: how far the audio has been slid so far, in samples. Each
  // move sends only what is new, one after the other through `paintChain`.
  slipSamples?: number
  /** A clip drag that changed the song while still held (onto another
   *  track, or a fade) opened an undo step, which the release closes once
   *  `paintChain` has landed, so the whole drag is one step. */
  historyGroupOpen?: boolean
  /** Where the drag has put the dragged clip, its length on a right-edge
   *  resize, and how far a group moved. The release sends these rather
   *  than reading the store, which a move to another track refreshes. */
  lastPositionTicks?: number
  lastLengthTicks?: number
  lastDeltaTicks?: number
  /** The position sent with the last move onto another track. */
  sentPositionTicks?: number
  /** The fades last sent during a fade drag, as [in, out]. */
  fadesSent?: [number, number]
}

/** Open a clip drag's undo step the first time it changes the song. */
function openDragStep(drag: DragState) {
  if (drag.historyGroupOpen) return
  drag.historyGroupOpen = true
  void useTrackStore.getState().beginHistoryGroup()
}

/** Close a clip drag's undo step, if it opened one, once what it sent has
 *  landed. */
function closeDragStep(drag: DragState, label: string, after: Promise<unknown>) {
  if (!drag.historyGroupOpen) return
  void after.finally(() => useTrackStore.getState().endHistoryGroup(label))
}

/** The song changed: closing it asks to save, and autosave picks it up.
 *  For playlist edits sent straight to the engine, which the track store's
 *  own commands do for themselves. */
function markEdited() {
  useProjectStore.getState().markDirty()
}

interface ContextMenuState {
  x: number
  y: number
  trackId: string
  clipId: string
}

// Cache keyed by `${sourceId}:${bucketTier}` so zooming in fetches a higher-resolution
// peak set instead of upscaling the existing one.
const waveformData = new Map<string, WavePeak[]>()

/** Up to this many buckets a file's waveform comes in one piece. Zoomed in
 *  further it comes in tiles of TILE buckets, only where the playlist shows
 *  it, so the detail at deep zoom (down to single samples, as FL draws it)
 *  costs what one screen needs. The whole-file waveform stands in while a
 *  tile is on its way. */
const WHOLE_TIER = 4096
const TILE = 2048
/** Tiles kept, oldest dropped first: a tile is about 100 kB. */
const MAX_TILES = 256
const waveformTiles = new Map<string, WavePeak[]>()
const tilesPending = new Set<string>()
const tileQueue = new Map<string, { sourceId: string; tier: number; tile: number }>()
let tileTimer: number | undefined
/** Set by the mounted playlist: draws again once tiles have arrived. */
let tilesArrived: (() => void) | null = null

/** Ask for one tile of a file's waveform. Asked while drawing, so the asks
 *  wait until the zoom and the scroll have settled, and only the latest
 *  zoom's tiles are fetched. */
function wantTile(sourceId: string, tier: number, tile: number) {
  const key = `${sourceId}:${tier}:${tile}`
  if (waveformTiles.has(key) || tilesPending.has(key)) return
  tileQueue.set(key, { sourceId, tier, tile })
  window.clearTimeout(tileTimer)
  tileTimer = window.setTimeout(fetchTiles, 120)
}

function fetchTiles() {
  const latest = new Map<string, number>()
  for (const t of tileQueue.values()) latest.set(t.sourceId, t.tier)
  const asks = [...tileQueue.entries()].filter(([, t]) => latest.get(t.sourceId) === t.tier)
  tileQueue.clear()
  for (const [key, t] of asks) {
    tilesPending.add(key)
    invoke<WavePeak[]>('get_waveform_peaks', { sourceId: t.sourceId, numBuckets: t.tier, first: t.tile * TILE, count: TILE })
      .then((peaks) => {
        waveformTiles.set(key, peaks)
        while (waveformTiles.size > MAX_TILES) {
          const oldest = waveformTiles.keys().next().value
          if (oldest === undefined) break
          waveformTiles.delete(oldest)
        }
        tilesArrived?.()
      })
      .catch(() => {})
      .finally(() => tilesPending.delete(key))
  }
}
const FADE_HANDLE_PX = 10
const HEADER_H = 14


/** Requests for waveform tiers that have not answered yet, so a quick
 *  zoom does not ask for the same file again with every wheel step. */
const waveformPending = new Set<string>()

/** How wide a clip's whole file is at this zoom, in pixels: what its
 *  waveform tier is chosen by. The clip's own width when the file's
 *  length is not known. */
function fileWidthPx(clip: ClipInfo, pxPerTick: number, bpm: number): number {
  if (clip.sourceFrames && clip.sourceRate) {
    const pxPerSec = pxPerTick * (bpm / 60) * PPQ
    const stretch = clip.timeFactor ?? (clip.stretchRatio > 0 ? clip.stretchRatio : 1)
    return (clip.sourceFrames / clip.sourceRate) * stretch * pxPerSec
  }
  return clip.length_ticks * pxPerTick
}

/** How many buckets to split a clip's file into at this zoom: about one
 *  per pixel, as a power of two so small zoom steps share one, and exactly
 *  one per sample once a sample is a pixel or wider, which is where the
 *  playlist draws the signal itself as a line. */
function clipTier(clip: ClipInfo, pxPerTick: number, bpm: number): number {
  const px = fileWidthPx(clip, pxPerTick, bpm)
  if (clip.sourceFrames && px >= clip.sourceFrames) return clip.sourceFrames
  return bucketTier(Math.max(100, Math.ceil(px)))
}

function bucketTier(desired: number): number {
  // Quantise to powers of two so we don't hammer the backend on tiny zoom changes.
  const clamped = Math.max(100, Math.min(1 << 24, Math.ceil(desired)))
  return 2 ** Math.ceil(Math.log2(clamped))
}

/** What an automation clip moves, as its header names it. */
function automationTargetLabel(target: AutomationTargetInfo): string {
  switch (target.kind) {
    case 'track_volume': return 'Volume'
    case 'track_pan': return 'Pan'
    case 'track_mute': return 'Mute'
    case 'plugin_param': return `Parameter ${target.paramId}`
    case 'send_level': return `Send ${target.sendIndex + 1}`
    default: return 'Automation'
  }
}

/** `#rrggbb` from an automation clip's ARGB colour. */
function argbHex(argb: number): string {
  return `#${(argb & 0xffffff).toString(16).padStart(6, '0')}`
}

/** An automation clip's value (0..1) at a tick inside it, straight between
 *  its points, as the curve is drawn. */
function automationValueAt(points: { tick: number; value: number }[], tick: number): number {
  if (points.length === 0) return 0.5
  if (tick <= points[0].tick) return points[0].value
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1]
    const b = points[i]
    if (tick <= b.tick) {
      const span = b.tick - a.tick
      return span > 0 ? a.value + ((tick - a.tick) / span) * (b.value - a.value) : b.value
    }
  }
  return points[points.length - 1].value
}

/** Where a clip's sound goes, for the hover note: "insert 4 (Kick)", its
 *  own insert, or the master. */
function insertOf(track: TrackWithClips, byId: Record<string, TrackWithClips>): string {
  const target = track.outputBus ? byId[track.outputBus] : undefined
  if (target) {
    if (target.kind === 'Master') return 'the master'
    return target.mixerNumber != null ? `insert ${target.mixerNumber} (${target.name})` : target.name
  }
  if (track.mixerNumber != null && track.mixerNumber > 0) return `its own insert, ${track.mixerNumber}`
  return 'the master'
}

// Hardwave clip palette — vibrant on near-black
const CLIP_COLORS = [
  '#DC2626', '#10B981', '#A855F7', '#F59E0B',
  '#3B82F6', '#EC4899', '#06B6D4', '#84CC16',
]

interface ArrangementProps {
  onSetHint?: (text: string) => void
}

// The playhead: bright red, a sharp 1px line over a glow (mockup style),
// with a caret at the top. PLAYHEAD_HALF puts the line on the position.
const PLAYHEAD_HALF = 5
const PLAYHEAD_STYLE: React.CSSProperties = {
  position: 'absolute', top: 0, left: 0, bottom: 0, width: PLAYHEAD_HALF * 2 + 1,
  pointerEvents: 'none', willChange: 'transform', display: 'none',
}
const PLAYHEAD_GLOW: React.CSSProperties = {
  position: 'absolute', top: 0, bottom: 0, left: PLAYHEAD_HALF - 2, width: 5, background: 'rgba(239,68,68,0.20)',
}
const PLAYHEAD_LINE: React.CSSProperties = {
  position: 'absolute', top: 0, bottom: 0, left: PLAYHEAD_HALF, width: 1, background: '#EF4444',
}
const PLAYHEAD_CARET: React.CSSProperties = {
  position: 'absolute', top: 0, left: 0, width: 0, height: 0,
  borderLeft: `${PLAYHEAD_HALF}px solid transparent`, borderRight: `${PLAYHEAD_HALF + 1}px solid transparent`,
  borderTop: '8px solid #EF4444',
}

/** Size a canvas's backing store for a CSS size; false when it already was. */
/** Persist a ripple move: the dragged clip, its group, and on each of their
 *  tracks every clip after them, moved by `delta`, as one undo step. The
 *  drag moved the dragged clips in the store already; the rest move here. */
async function commitRippleMove(drag: DragState, delta: number) {
  const store = useTrackStore.getState()
  const members = [
    { clipId: drag.clipId, trackId: drag.trackId, origPos: drag.originalPositionTicks },
    ...(drag.groupMoveOriginals ?? []),
  ]
  const byTrack = new Map<string, typeof members>()
  for (const m of members) byTrack.set(m.trackId, [...(byTrack.get(m.trackId) ?? []), m])
  await store.beginHistoryGroup()
  try {
    for (const [trackId, moved] of byTrack) {
      const track = useTrackStore.getState().tracks.find(t => t.id === trackId)
      if (!track) continue
      const ids = new Set(moved.map(m => m.clipId))
      const from = Math.min(...moved.map(m => m.origPos))
      // Where the other clips were before the drag: the store holds them
      // there still, since only the dragged ones moved.
      const shifts = rippleMoveShifts(track.clips ?? [], ids, from, delta)
      const all = [
        ...moved.map(m => [m.clipId, Math.max(0, m.origPos + delta)] as [string, number]),
        ...shifts.entries(),
      ]
      // Moving later: the last clip first, so none lands on one yet to move;
      // moving earlier: the first first.
      all.sort((a, b) => (delta > 0 ? b[1] - a[1] : a[1] - b[1]))
      for (const [clipId, newPositionTicks] of all) {
        await invoke('move_clip', { trackId, clipId, newPositionTicks })
      }
    }
  } finally {
    markEdited()
    await store.endHistoryGroup(members.length === 1 ? 'Move clip, and the clips after it' : `Move ${members.length} clips, and the clips after them`)
  }
  await useTrackStore.getState().fetchTracks()
}

/** A MIDI clip's notes inside its body (see the call in drawClip). */
function drawNotePreview(
  ctx: CanvasRenderingContext2D,
  notes: [number, number, number][],
  color: string,
  muted: boolean,
  x: number, y: number, w: number, h: number,
  pxPerTick: number,
) {
  let lo = 127, hi = 0
  for (const n of notes) { if (n[2] < lo) lo = n[2]; if (n[2] > hi) hi = n[2] }
  // At least an octave of room, so a part on one note sits mid-clip
  // rather than as a bar along the top.
  const span = Math.max(12, hi - lo + 1)
  const mid = (lo + hi) / 2
  const low = mid - span / 2
  const pad = 2
  const rowH = Math.max(1, Math.min(4, (h - pad * 2) / span))
  ctx.save()
  ctx.beginPath()
  ctx.rect(x + 1, y, w - 2, h - 1)
  ctx.clip()
  ctx.fillStyle = muted ? '#52525b' : mixHex(color, '#ffffff', 0.35)
  ctx.globalAlpha = muted ? 0.5 : 0.9
  for (const [start, len, pitch] of notes) {
    const nx = x + start * pxPerTick
    if (nx > x + w) continue
    const nw = Math.max(1, len * pxPerTick - 1)
    const ny = y + pad + (1 - (pitch - low + 0.5) / span) * (h - pad * 2) - rowH / 2
    ctx.fillRect(nx, ny, Math.min(nw, x + w - nx), rowH)
  }
  ctx.restore()
}

function fitCanvas(canvas: HTMLCanvasElement, w: number, h: number, dpr: number): boolean {
  const pw = Math.round(w * dpr)
  const ph = Math.round(h * dpr)
  if (canvas.width === pw && canvas.height === ph) return false
  canvas.width = pw
  canvas.height = ph
  return true
}

export function Arrangement({ onSetHint }: ArrangementProps = {}) {
  useLanguage()
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<DragState | null>(null)
  const [, forceRender] = useState(0)
  /// Vertical scroll offset in pixels — how far down the user has
  /// scrolled the track list. Plain wheel input on the playlist
  /// (no Ctrl modifier) adjusts this. Clamped to
  /// `0..(content_height - viewport_height)` in the wheel handler.
  /// All canvas draws and hit-tests subtract this from their logical
  /// Y so the visual band of tracks slides up under the ruler.
  // Shared with the track name column, which scrolls with it.
  const verticalScroll = usePlaylistScrollStore(s => s.y)
  const setVerticalScroll = useCallback((next: number | ((v: number) => number)) => {
    const store = usePlaylistScrollStore.getState()
    store.setY(typeof next === 'function' ? next(store.y) : next)
  }, [])
  // Horizontal pan, in pixels. Until this existed the arrangement's scroll
  // position was recomputed from the playhead on every draw, so
  // follow-playhead wasn't a mode — it was the only behaviour, and the
  // timeline could not be navigated by hand at all. Mirrors the piano
  // roll's scrollX + followPlayhead pair.
  const [scrollX, setScrollX] = useState(0)
  // Following the playhead is one saved switch, shared with the piano roll
  // and off by default. The playlist used to scroll itself the moment the
  // song played, which drags the view away from the part being worked on.
  // Panning by hand turns it off, the same way dragging a scrollbar does in
  // every other DAW, and the FOLLOW button turns it back on.
  const followPlayhead = useGeneralPrefsStore(s => s.followPlayhead)
  const followMode = useGeneralPrefsStore(s => s.followMode)
  const rulerUnits = useGeneralPrefsStore(s => s.rulerUnits)
  const timecodeFps = useGeneralPrefsStore(s => s.timecodeFps)
  // Named stretches of the song, drawn along the ruler so the arrangement
  // can be read at a glance instead of counted in bars.
  const [sections, setSections] = useState<{ id: string; name: string; startTicks: number; endTicks: number }[]>([])
  useEffect(() => {
    const load = () => {
      invoke<{ id: string; name: string; startTicks: number; endTicks: number }[]>('list_sections')
        .then(setSections)
        .catch(() => setSections([]))
    }
    load()
    window.addEventListener('daw:sectionsChanged', load)
    return () => window.removeEventListener('daw:sectionsChanged', load)
  }, [])
  const setFollowPlayhead = useGeneralPrefsStore(s => s.setFollowPlayhead)
  /// FL Studio convention: right-mouse-button + drag = 2D pan (vertical
  /// scroll + horizontal pan via setting the playhead-derived offset).
  /// We track that here so onContextMenu can suppress its menu when the
  /// drag has moved past the threshold — i.e. when the user clearly
  /// intended a pan, not a menu open.
  const rightPanRef = useRef<{
    startY: number
    startVerticalScroll: number
    moved: boolean
  } | null>(null)

  const {
    tracks, selectedClipId, selectedClipIds, selectClip, toggleClipSelection, clearSelection,
    moveClipLocal, placeClipsLocal, resizeClipLocal, commitClipDrag, commitClipTrimStart,
    getWaveformPeaks, duplicateClip, splitClip, deleteClip, setClipFades,
    setClipFadeCurves, toggleClipReverse, setClipGain, setClipPitch, setClipStretch,
  } = useTrackStore()
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null)
  // The automation clip menu: what a right-click on one offers.
  const [autoMenu, setAutoMenu] = useState<{ x: number; y: number; trackId: string; clipId: string; name: string } | null>(null)
  const [tuneTarget, setTuneTarget] = useState<{ trackId: string; clipId: string } | null>(null)
  const [stemsTarget, setStemsTarget] = useState<{ trackId: string; clipId: string; name: string } | null>(null)
  // Where the other person in the room is working, drawn as a line of
  // its own so it cannot be mistaken for the playhead.
  const peer = useCollabStore(s => s.peer)
  const [spectralTarget, setSpectralTarget] = useState<{ trackId: string; clipId: string } | null>(null)
  const [markerCtx, setMarkerCtx] = useState<{ x: number; y: number; markerId: string | null; sectionId: string | null; tick: number } | null>(null)
  const [renamingMarker, setRenamingMarker] = useState<{ id: string; draft: string } | null>(null)
  // Only what the arrangement draws from. The playhead moves 30 times a
  // second while playing; it is drawn on a canvas of its own (below), so
  // the arrangement neither re-renders nor redraws every clip for it.
  const {
    bpm, sampleRate, setPosition, looping, loopStart, loopEnd,
    setLoop, toggleLoop,
    trackHeight, setTrackHeight, snapValue, snapEnabled, horizontalZoom, setHorizontalZoom,
    editCursorTicks, setEditCursor,
    punchEnabled, punchInTicks, punchOutTicks, setPunchIn, setPunchOut, clearPunch, setPunchRangeFromLoop,
  } = useTransportStore(useShallow((s) => ({
    bpm: s.bpm, sampleRate: s.sampleRate, setPosition: s.setPosition, looping: s.looping,
    loopStart: s.loopStart, loopEnd: s.loopEnd, setLoop: s.setLoop, toggleLoop: s.toggleLoop,
    trackHeight: s.trackHeight, setTrackHeight: s.setTrackHeight, snapValue: s.snapValue,
    snapEnabled: s.snapEnabled, horizontalZoom: s.horizontalZoom, setHorizontalZoom: s.setHorizontalZoom,
    editCursorTicks: s.editCursorTicks,
    setEditCursor: s.setEditCursor, punchEnabled: s.punchEnabled,
    punchInTicks: s.punchInTicks, punchOutTicks: s.punchOutTicks, setPunchIn: s.setPunchIn,
    setPunchOut: s.setPunchOut, clearPunch: s.clearPunch, setPunchRangeFromLoop: s.setPunchRangeFromLoop,
  })))
  const { markers: songMarkers, addMarker, removeMarker, updateMarker, jumpToNext, jumpToPrev } = useMarkerStore()
  // Tempo and signature flags come from the tempo map, so they show what
  // plays. The ones the marker list still holds from before never changed
  // the song and are not drawn.
  const tempoEntries = useTempoMapStore(s => s.entries)
  // The tempo lane, under the ruler. With it shown, the tempo is drawn
  // there, so the ruler keeps only the signature changes.
  const tempoLaneShown = useGeneralPrefsStore(s => s.tempoLaneShown)
  const laneHot = useTempoMapStore(s => s.laneHot)
  const laneScaleHeld = useTempoMapStore(s => s.laneScale)
  RULER_HEIGHT = BAR_RULER_HEIGHT + (tempoLaneShown ? TEMPO_LANE_HEIGHT : 0)
  const markers = useMemo(() => [
    ...songMarkers.filter(m => m.kind !== 'tempo' && m.kind !== 'timesig'),
    ...tempoFlags(tempoEntries).filter(f => !(tempoLaneShown && f.kind === 'tempo')),
  ].sort((a, b) => a.tick - b.tick), [songMarkers, tempoEntries, tempoLaneShown])
  // Samples, where the engine counts, and ticks, where the playlist lays
  // the song out, convert through the tempo map: the playhead, a click on
  // the ruler and the loop are where the song is at that moment, not where
  // one tempo would put it. Read from a ref so the handlers made earlier
  // still convert with the map as it is now.
  const tempoRef = useRef(tempoEntries)
  tempoRef.current = tempoEntries
  const tickAtPosition = (samples: number) =>
    tickAtSample(tempoRef.current, samples, useTransportStore.getState().sampleRate || 48000)
  const positionAtTick = (tick: number) =>
    sampleAtTick(tempoRef.current, tick, useTransportStore.getState().sampleRate || 48000)
  const tempoActions = useTempoMapStore.getState
  // Bar layout from the project's tempo map, so a signature change part-way
  // through the song moves the bar lines and the bar numbers with it.
  const meterSegments = useTempoMapStore(s => s.segments)
  const refreshTempoMap = useTempoMapStore(s => s.refresh)
  const clipToGroup = useClipGroupStore(s => s.clipToGroup)
  const groupColors = useClipGroupStore(s => s.groupColors)
  const groupClipsAction = useClipGroupStore(s => s.groupClips)
  const ungroupClipAction = useClipGroupStore(s => s.ungroupClip)

  // Every track but the master, in the song's order: what plays. Where
  // its clips show is up to the rows.
  const audioTracks = useMemo(() => tracks.filter(t => t.kind !== 'Master'), [tracks])
  // The playlist's rows (FL's playlist tracks) and what sits on each,
  // sorted by time, so a frame draws and a click tests only what is in
  // view. A clip sits on its own row whichever channel plays it.
  const rowIndex = useMemo(() => rowIndexOf(tracks), [tracks])
  // Each track's place among the tracks the playlist used to list, for
  // the clip colour a track's clips have always had.
  const colourPlace = useMemo(() => trackPlaces(tracks), [tracks])
  const playlistRows = usePlaylistRowsStore(s => s.rows)
  const silent = useMemo(() => silentRows(playlistRows), [playlistRows])
  const rowCount = shownRowCount(rowIndex, playlistRows.length)
  const contentH = rowCount * trackHeight
  // Tell the name column how far the playlist can scroll, so a wheel over
  // the names stops where the grid stops.
  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    const publish = () => {
      const viewportH = container.clientHeight - RULER_HEIGHT
      usePlaylistScrollStore.getState().setMax(Math.max(0, contentH - viewportH))
    }
    publish()
    if (typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(publish)
    ro.observe(container)
    return () => ro.disconnect()
  }, [contentH])
  const PIXELS_PER_SECOND = PIXELS_PER_SECOND_BASE * horizontalZoom
  // The playlist is in bars and beats at a fixed scale: the opening tempo's
  // beat is as wide as it always was. It used to scale with the tempo the
  // transport showed, so playing through a tempo change stretched the whole
  // playlist under the playhead.
  // Held while a tempo point is dragged, so dragging the opening tempo
  // does not zoom the playlist under the pointer.
  const layoutRef = useRef(layoutBpm(tempoEntries, bpm))
  if (laneScaleHeld === null) layoutRef.current = layoutBpm(tempoEntries, bpm)
  const layoutTempo = layoutRef.current
  const beatsPerSecond = layoutTempo / 60
  const pixelsPerBeat = PIXELS_PER_SECOND / beatsPerSecond
  const pixelsPerTick = pixelsPerBeat / PPQ
  const snapTicks = snapToTicks(snapValue, snapEnabled)
  const applySnap = (ticks: number): number => snapTicks > 0 ? Math.round(ticks / snapTicks) * snapTicks : ticks

  // FL Studio multi-touch gestures on the playlist canvas (manual page
  // "The User Interface" — Multi-Touch section):
  //   - pinch    → horizontal zoom (same as Ctrl+wheel)
  //   - 2-finger → 2D pan (mirrors right-mouse-drag for mouse users)
  //   - 2-tap    → not used here (single-tap already selects clips)
  // The hook attaches native listeners so we keep pinch + multi-finger
  // logic out of every individual mousedown branch.
  useMultiTouchGestures(containerRef, {
    onPinch: (factor) => {
      // factor is current/start; bound to a per-event 1.02..0.98
      // range so a quick spread doesn't 4x the zoom instantly.
      const clamped = Math.max(0.7, Math.min(1.5, factor))
      setHorizontalZoom(horizontalZoom * clamped)
    },
    onPan: (_dx, dy) => {
      // Vertical pan only — horizontal "pan" is derived from playhead
      // position via getScrollOffset, not a user-settable offset.
      const container = containerRef.current
      if (!container) return
      const viewportH = container.clientHeight - RULER_HEIGHT
      const maxScroll = Math.max(0, contentH - viewportH)
      setVerticalScroll((v) => Math.max(0, Math.min(maxScroll, v - dy)))
    },
  })

  // The grid cannot be drawn before the tempo map is known, and the map only
  // changes when the project or the tempo dialog changes it, so one read at
  // mount is enough. A signature change made later refreshes the store from
  // wherever it was made.
  useEffect(() => { refreshTempoMap() }, [refreshTempoMap])

  // Load waveforms. Multi-zoom tiering: cache by (sourceId, tier) so zooming in refetches
  // a higher-resolution peak set rather than stretching the existing buckets.
  // Asked for once the zoom has settled (a quick zoom used to ask for every
  // file at every step), each tier once, and the redraws they cause come
  // together in one frame.
  const redrawQueued = useRef(false)
  useEffect(() => {
    const timer = window.setTimeout(() => {
      for (const track of audioTracks) {
        for (const clip of track.clips) {
          if (!clip.source_id) continue
          const tier = Math.min(WHOLE_TIER, clipTier(clip, pixelsPerTick, layoutTempo))
          const key = `${clip.source_id}:${tier}`
          if (waveformData.has(key) || waveformPending.has(key)) continue
          waveformPending.add(key)
          getWaveformPeaks(clip.source_id, tier)
            .then((peaks) => {
              waveformData.set(key, peaks)
              if (!redrawQueued.current) {
                redrawQueued.current = true
                requestAnimationFrame(() => {
                  redrawQueued.current = false
                  // The draw does not run on a render of its own: ask it.
                  drawMainRef.current()
                })
              }
            })
            .finally(() => waveformPending.delete(key))
        }
      }
    }, 150)
    return () => window.clearTimeout(timer)
  }, [tracks, horizontalZoom])

  // Draw. Kept in a ref so following the playhead can redraw without a
  // React render; the effect below re-makes it when what it draws changes.
  const drawMainRef = useRef<() => void>(() => {})
  const drawPlayheadRef = useRef<() => void>(() => {})
  // Tiles of a deep-zoomed waveform arrive after the draw asked for them.
  useEffect(() => {
    tilesArrived = () => {
      if (redrawQueued.current) return
      redrawQueued.current = true
      requestAnimationFrame(() => {
        redrawQueued.current = false
        drawMainRef.current()
      })
    }
    return () => { tilesArrived = null }
  }, [])
  useEffect(() => {
   const draw = () => {
    const canvas = canvasRef.current
    const container = containerRef.current
    if (!canvas || !container) return

    const dpr = window.devicePixelRatio || 1
    const rect = container.getBoundingClientRect()
    // Resizing a canvas reallocates it, so only when the size changed.
    if (fitCanvas(canvas, rect.width, rect.height, dpr)) {
      canvas.style.width = `${rect.width}px`
      canvas.style.height = `${rect.height}px`
    }

    const ctx = canvas.getContext('2d')
    if (!ctx) return
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.clearRect(0, 0, rect.width, rect.height)
    const { positionSamples } = useTransportStore.getState()

    const w = rect.width
    const h = rect.height

    // Publish what a fit needs: the width available and where the song ends.
    // View > Fit song to window can then work from a menu without knowing
    // either. zoomToFit used to just set the zoom to 1, which is a reset
    // wearing a fit's name, and nothing ever called it.
    {
      let songEnd = 0
      for (const t of audioTracks) {
        for (const c of t.clips) {
          songEnd = Math.max(songEnd, c.position_ticks + c.length_ticks)
        }
      }
      useTransportStore.getState().setPlaylistMetrics(w, songEnd)
    }

    const scrollOffset = followPlayhead
      ? followScroll(tickAtPosition(positionSamples) * pixelsPerTick, w, followMode)
      : scrollX

    // Background: neutral near-black, the same base as the rest of the
    // shell (rework.css). It was a purple tint (#15091a).
    ctx.fillStyle = '#0e0e11'
    ctx.fillRect(0, 0, w, h)

    // Horizontal track-row separators — match mockup's .fl-pl-canvas
    // background-image: rgba(0,0,0,.55) line every --row-h px. Drawn before
    // beat lines so vertical bar lines appear on top. Separators are
    // offset by RULER_HEIGHT so they line up with the actual track rows
    // which start under the ruler band — otherwise every row's separator
    // sits 22 px too high relative to its row content.
    if (trackHeight > 0) {
      ctx.strokeStyle = 'rgba(0,0,0,0.55)'
      ctx.lineWidth = 1
      // Track row separators scroll vertically with the user's wheel
      // offset. Only draw rows whose y lands within the visible band
      // below the ruler — avoid drawing thousands of off-screen rows.
      const firstVisible = Math.floor(verticalScroll / trackHeight)
      const lastVisible = firstVisible + Math.ceil((h - RULER_HEIGHT) / trackHeight) + 1
      for (let r = firstVisible; r <= lastVisible; r++) {
        const y = Math.floor(RULER_HEIGHT + r * trackHeight - verticalScroll) + 0.5
        if (y > h) break
        if (y < RULER_HEIGHT) continue
        ctx.beginPath()
        ctx.moveTo(0, y)
        ctx.lineTo(w, y)
        ctx.stroke()
      }
    }

    // Top ruler band — bar numbers drawn in the SAME pixel space as the
    // grid lines below. Previously the ruler was an HTML element in
    // HwApp.tsx that flex-distributed its markers regardless of zoom +
    // tempo, so the numbers never lined up with the canvas vertical
    // grid lines. Drawing inside the canvas guarantees alignment by
    // construction.
    if (BAR_RULER_HEIGHT > 0) {
      ctx.fillStyle = '#040406'
      ctx.fillRect(0, 0, w, BAR_RULER_HEIGHT)
      ctx.strokeStyle = 'rgba(255, 255, 255, 0.08)'
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(0, BAR_RULER_HEIGHT - 0.5)
      ctx.lineTo(w, BAR_RULER_HEIGHT - 0.5)
      ctx.stroke()
    }

    // Beat grid — HQ pixel-snapped, mockup palette. Walked from the tempo
    // map's meter segments rather than one bar length, so bars of 3/4 and
    // 7/8 in the same song are each drawn their own width.
    const visibleFromTick = pixelsPerTick > 0 ? Math.max(0, scrollOffset / pixelsPerTick) : 0
    const visibleToTick = pixelsPerTick > 0 ? (scrollOffset + w) / pixelsPerTick : 0
    const lines = gridLines(meterSegments, visibleFromTick, visibleToTick)
    for (const line of lines) {
      const x = Math.floor(line.tick * pixelsPerTick - scrollOffset) + 0.5
      if (x < -1 || x > w + 1) continue
      ctx.strokeStyle = line.isBar ? 'rgba(255,255,255,0.11)' : 'rgba(255,255,255,0.04)'
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(x, 0)
      ctx.lineTo(x, h)
      ctx.stroke()
    }

    // Bar number labels in the ruler band. Taken from the same lines as the
    // grid, so the numbers cannot drift from the bars they sit over, which is
    // what happened when they were counted from one fixed bar width. Labels
    // are dropped rather than drawn on top of each other when the zoom makes
    // the bars narrow.
    if (BAR_RULER_HEIGHT > 0) {
      const MIN_LABEL_GAP_PX = 34
      ctx.fillStyle = 'rgba(180, 180, 200, 0.7)'
      ctx.font = '600 10px "JetBrains Mono", ui-monospace, Consolas, monospace'
      ctx.textBaseline = 'middle'
      let lastLabelX = -Infinity
      // Bars for music; minutes and seconds or timecode for picture and
      // sound design, which had no way to read this timeline at all. The
      // seconds come from the same tick the bar line sits on, through the
      // project tempo, so the two readings never disagree.
      const label = (line: { tick: number; bar: number }): string =>
        rulerLabel(line.tick, line.bar, rulerUnits, tempoEntries, timecodeFps, sampleRate || 48000)
      // Time labels are wider than a bar number, so they need more room
      // before they start overlapping.
      const gap = labelGap(rulerUnits, MIN_LABEL_GAP_PX)
      for (const line of lines) {
        if (!line.isBar) continue
        const x = Math.floor(line.tick * pixelsPerTick - scrollOffset) + 4
        if (x < -20 || x > w) continue
        if (x - lastLabelX < gap) continue
        lastLabelX = x
        ctx.fillText(label(line), x, BAR_RULER_HEIGHT / 2 + 1)
      }

      // Section names, over the bar numbers they cover. Drawn as a tinted
      // band so the shape of the arrangement reads without counting bars.
      for (const section of sections) {
        const x1 = Math.floor(section.startTicks * pixelsPerTick - scrollOffset)
        const x2 = Math.floor(section.endTicks * pixelsPerTick - scrollOffset)
        if (x2 < 0 || x1 > w) continue
        ctx.fillStyle = 'rgba(220,38,38,0.16)'
        ctx.fillRect(x1, 0, Math.max(1, x2 - x1), BAR_RULER_HEIGHT)
        ctx.strokeStyle = 'rgba(220,38,38,0.5)'
        ctx.beginPath()
        ctx.moveTo(x1 + 0.5, 0)
        ctx.lineTo(x1 + 0.5, BAR_RULER_HEIGHT)
        ctx.stroke()
        ctx.fillStyle = 'rgba(255,220,220,0.9)'
        ctx.fillText(section.name, x1 + 5, BAR_RULER_HEIGHT / 2 + 1)
      }
      ctx.fillStyle = 'rgba(180, 180, 200, 0.7)'
    }

    // Sub-beat snap grid — only when snap is finer than 1/4 and lines won't be too dense
    if (snapTicks > 0 && snapTicks < PPQ) {
      const snapPx = snapTicks * pixelsPerTick
      if (snapPx >= 4) {
        const startN = Math.floor(scrollOffset / snapPx)
        const endN = startN + Math.ceil(w / snapPx) + 2
        ctx.strokeStyle = 'rgba(255,255,255,0.015)'
        ctx.lineWidth = 0.5
        for (let i = startN; i < endN; i++) {
          const x = i * snapPx - scrollOffset
          if (x < -1 || x > w + 1) continue
          // Don't redraw over beat lines
          if (Math.abs((i * snapTicks) % PPQ) < 0.5) continue
          ctx.beginPath()
          ctx.moveTo(x, RULER_HEIGHT)
          ctx.lineTo(x, h)
          ctx.stroke()
        }
      }
    }

    // (Earlier code drew alternating opaque row backgrounds for 500
    // slots here. That fillRect ran AFTER the bar/beat grid and the
    // row-separator strokes, painting solid #08080c/#0b0b10 over both
    // and leaving the empty playlist with just flat alternating
    // stripes — no visible bars, no visible beats. The mockup has no
    // alternating row colours: it's a single #15091a fill plus the
    // horizontal row-separator and vertical bar/beat lines drawn
    // above. Removing the loop exposes the grid that was always being
    // drawn underneath.)

    // Clips, row by row: only the rows in view, and on each only what
    // reaches into the visible bars. A row's clips can belong to any
    // channel; each is drawn in its channel's colour. A clip on a row
    // that is switched off (muted, or not soloed while another is) is
    // drawn faded, as FL greys out a muted playlist track.
    {
      const firstRow = Math.max(0, Math.floor(verticalScroll / trackHeight))
      const lastRow = Math.min(rowCount - 1, Math.floor((verticalScroll + h - RULER_HEIGHT) / trackHeight))
      const fromTick = pixelsPerTick > 0 ? scrollOffset / pixelsPerTick : 0
      const toTick = pixelsPerTick > 0 ? (scrollOffset + w) / pixelsPerTick : 0
      // Take lanes: with several takes spread out, a track's band is split
      // into one strip per lane so the passes can be compared instead of
      // hiding behind each other.
      const laneCounts = new Map<string, number>()
      const lanesOf = (track: TrackWithClips) => {
        let n = laneCounts.get(track.id)
        if (n === undefined) {
          n = track.clips.reduce((max, c) => Math.max(max, (c.lane ?? 0) + 1), 1)
          laneCounts.set(track.id, n)
        }
        return n
      }
      for (let r = firstRow; r <= lastRow; r++) {
        const y = RULER_HEIGHT + r * trackHeight - verticalScroll
        const items = itemsInView(rowIndex, r, fromTick, toTick)
        if (items.length === 0) continue
        for (const item of items) {
          if (item.kind === 'auto') {
            drawAutomationClip(ctx, item, y, scrollOffset, w, pixelsPerTick)
            continue
          }
          const place = colourPlace[item.trackIndex]
          const color = item.clip.color || CLIP_COLORS[Math.max(0, place) % CLIP_COLORS.length]
          drawClip(ctx, item.clip, color, y, scrollOffset, w, pixelsPerTick, item.clip.lane ?? 0, lanesOf(item.track))
        }
        // A row switched off: its clips are veiled, as they are silent.
        if (silent(r)) {
          ctx.fillStyle = 'rgba(14,14,17,0.62)'
          for (const item of items) {
            const x = item.start * pixelsPerTick - scrollOffset
            ctx.fillRect(x, y + 1, (item.end - item.start) * pixelsPerTick, trackHeight - 2)
          }
        }
        // Crossfades: an overlap between two clips of one channel on the
        // row is shaded, as they blend there.
        for (let j = 1; j < items.length; j++) {
          const cur = items[j]
          if (cur.kind !== 'clip') continue
          for (let k = j - 1; k >= 0; k--) {
            const prev = items[k]
            if (prev.kind !== 'clip' || prev.track.id !== cur.track.id) continue
            if (cur.start < prev.end) {
              const xStart = cur.start * pixelsPerTick - scrollOffset
              const width = (Math.min(prev.end, cur.end) - cur.start) * pixelsPerTick
              if (xStart + width >= 0 && xStart <= w) {
                ctx.save()
                ctx.fillStyle = 'rgba(20, 184, 166, 0.12)'
                ctx.fillRect(xStart, y + 2, width, trackHeight - 4)
                ctx.strokeStyle = 'rgba(20, 184, 166, 0.45)'
                ctx.lineWidth = 1
                ctx.beginPath()
                ctx.moveTo(xStart, y + 2)
                ctx.lineTo(xStart + width, y + trackHeight - 2)
                ctx.moveTo(xStart + width, y + 2)
                ctx.lineTo(xStart, y + trackHeight - 2)
                ctx.stroke()
                ctx.restore()
              }
            }
            break
          }
        }
      }
    }

    // The tempo lane, over anything scrolled up under it.
    if (tempoLaneShown) {
      drawTempoLane(ctx, {
        entries: tempoEntries, top: BAR_RULER_HEIGHT, width: w, pxPerTick: pixelsPerTick, scrollOffset,
        scale: laneScaleHeld ?? laneScale(tempoEntries), hot: laneHot, lines,
      })
    }

    // Loop region overlay
    if (looping && loopEnd > loopStart && sampleRate > 0) {
      const loopX1 = tickAtPosition(loopStart) * pixelsPerTick - scrollOffset
      const loopX2 = tickAtPosition(loopEnd) * pixelsPerTick - scrollOffset

      // Shaded region across all tracks
      ctx.fillStyle = 'rgba(220, 38, 38, 0.06)'
      ctx.fillRect(loopX1, RULER_HEIGHT, loopX2 - loopX1, h - RULER_HEIGHT)

      // Ruler highlight
      ctx.fillStyle = 'rgba(220, 38, 38, 0.15)'
      ctx.fillRect(loopX1, 0, loopX2 - loopX1, BAR_RULER_HEIGHT)

      // Loop boundary lines
      ctx.strokeStyle = 'rgba(220, 38, 38, 0.5)'
      ctx.lineWidth = 1.5
      ctx.setLineDash([4, 3])
      ctx.beginPath()
      ctx.moveTo(loopX1, 0)
      ctx.lineTo(loopX1, h)
      ctx.moveTo(loopX2, 0)
      ctx.lineTo(loopX2, h)
      ctx.stroke()
      ctx.setLineDash([])

      // Loop marker triangles on ruler
      ctx.fillStyle = '#DC2626'
      // Start marker — right-pointing triangle
      ctx.beginPath()
      ctx.moveTo(loopX1, 0)
      ctx.lineTo(loopX1 + 6, BAR_RULER_HEIGHT / 2)
      ctx.lineTo(loopX1, BAR_RULER_HEIGHT)
      ctx.closePath()
      ctx.fill()
      // End marker — left-pointing triangle
      ctx.beginPath()
      ctx.moveTo(loopX2, 0)
      ctx.lineTo(loopX2 - 6, BAR_RULER_HEIGHT / 2)
      ctx.lineTo(loopX2, BAR_RULER_HEIGHT)
      ctx.closePath()
      ctx.fill()
    }

    // Punch range overlay — teal brackets on the ruler, tinted strip below
    if (punchEnabled && punchInTicks != null && punchOutTicks != null && punchOutTicks > punchInTicks) {
      const pX1 = punchInTicks * pixelsPerTick - scrollOffset
      const pX2 = punchOutTicks * pixelsPerTick - scrollOffset

      ctx.fillStyle = 'rgba(20, 184, 166, 0.05)'
      ctx.fillRect(pX1, RULER_HEIGHT, pX2 - pX1, h - RULER_HEIGHT)

      ctx.fillStyle = 'rgba(20, 184, 166, 0.14)'
      ctx.fillRect(pX1, 0, pX2 - pX1, BAR_RULER_HEIGHT)

      ctx.strokeStyle = 'rgba(20, 184, 166, 0.7)'
      ctx.lineWidth = 1
      ctx.setLineDash([2, 2])
      ctx.beginPath()
      ctx.moveTo(pX1, 0); ctx.lineTo(pX1, h)
      ctx.moveTo(pX2, 0); ctx.lineTo(pX2, h)
      ctx.stroke()
      ctx.setLineDash([])

      // Square brackets at the ruler top
      ctx.strokeStyle = '#14B8A6'
      ctx.lineWidth = 1.5
      ctx.beginPath()
      ctx.moveTo(pX1 + 6, 2); ctx.lineTo(pX1, 2); ctx.lineTo(pX1, BAR_RULER_HEIGHT - 2); ctx.lineTo(pX1 + 6, BAR_RULER_HEIGHT - 2)
      ctx.moveTo(pX2 - 6, 2); ctx.lineTo(pX2, 2); ctx.lineTo(pX2, BAR_RULER_HEIGHT - 2); ctx.lineTo(pX2 - 6, BAR_RULER_HEIGHT - 2)
      ctx.stroke()
    }

    // The other person in the room: a violet line where they last
    // clicked, with their name on the ruler, so "look at bar 33" needs
    // no saying. Violet because red is the playhead and the two must
    // never be confused.
    if (peer) {
      const peerX = Math.floor(peer.tick * pixelsPerTick - scrollOffset) + 0.5
      if (peerX >= 0 && peerX <= w) {
        ctx.strokeStyle = 'rgba(168,85,247,0.85)'
        ctx.lineWidth = 1
        ctx.setLineDash([4, 3])
        ctx.beginPath()
        ctx.moveTo(peerX, RULER_HEIGHT)
        ctx.lineTo(peerX, h)
        ctx.stroke()
        ctx.setLineDash([])
        ctx.font = '600 10px Inter, system-ui, sans-serif'
        const label = peer.name
        const width = ctx.measureText(label).width + 10
        // To the left of the line, so the bar number that starts at the
        // line stays readable; flipped to the right near the left edge.
        const labelX = peerX - width >= 0 ? peerX - width : peerX
        ctx.fillStyle = 'rgba(168,85,247,0.95)'
        ctx.fillRect(labelX, 3, width, BAR_RULER_HEIGHT - 6)
        ctx.fillStyle = '#fff'
        ctx.fillText(label, labelX + 5, BAR_RULER_HEIGHT - 8)
      }
    }

    // Markers on ruler — colored flag + label. Tempo/time-sig markers render as
    // pill badges rather than flags so they read as metadata, not navigation points.
    for (const m of markers) {
      const mx = m.tick * pixelsPerTick - scrollOffset
      if (mx < -60 || mx > w + 2) continue
      ctx.save()
      ctx.strokeStyle = m.color
      ctx.lineWidth = 1
      ctx.setLineDash([2, 3])
      ctx.beginPath()
      ctx.moveTo(mx, RULER_HEIGHT)
      ctx.lineTo(mx, h)
      ctx.stroke()
      ctx.setLineDash([])

      const kind = m.kind ?? 'generic'
      if (kind === 'tempo' || kind === 'timesig') {
        const text = kind === 'tempo'
          ? `♩=${m.bpm != null ? (m.bpm % 1 === 0 ? m.bpm.toFixed(0) : m.bpm.toFixed(1)) : '?'}`
          : `${m.timeSigNum ?? '?'}/${m.timeSigDen ?? '?'}`
        ctx.font = '9px Inter, ui-sans-serif, sans-serif'
        const padX = 4
        const textW = ctx.measureText(text).width
        const pillW = Math.ceil(textW + padX * 2)
        const pillH = Math.min(BAR_RULER_HEIGHT - 3, 14)
        const pillY = 2
        ctx.fillStyle = m.color
        ctx.beginPath()
        const r = 3
        ctx.moveTo(mx + r, pillY)
        ctx.lineTo(mx + pillW - r, pillY)
        ctx.quadraticCurveTo(mx + pillW, pillY, mx + pillW, pillY + r)
        ctx.lineTo(mx + pillW, pillY + pillH - r)
        ctx.quadraticCurveTo(mx + pillW, pillY + pillH, mx + pillW - r, pillY + pillH)
        ctx.lineTo(mx + r, pillY + pillH)
        ctx.quadraticCurveTo(mx, pillY + pillH, mx, pillY + pillH - r)
        ctx.lineTo(mx, pillY + r)
        ctx.quadraticCurveTo(mx, pillY, mx + r, pillY)
        ctx.closePath()
        ctx.fill()
        if (renamingMarker?.id !== m.id) {
          ctx.fillStyle = '#0a0a0f'
          ctx.textBaseline = 'middle'
          ctx.fillText(text, mx + padX, pillY + pillH / 2 + 0.5)
        }
      } else {
        // Flag on ruler
        ctx.fillStyle = m.color
        ctx.beginPath()
        ctx.moveTo(mx, 1)
        ctx.lineTo(mx + 10, 1)
        ctx.lineTo(mx + 10, BAR_RULER_HEIGHT - 10)
        ctx.lineTo(mx + 4, BAR_RULER_HEIGHT - 6)
        ctx.lineTo(mx, BAR_RULER_HEIGHT - 10)
        ctx.closePath()
        ctx.fill()

        if (renamingMarker?.id !== m.id) {
          ctx.fillStyle = '#d4d4d8'
          ctx.font = '9px Inter, ui-sans-serif, sans-serif'
          ctx.textBaseline = 'middle'
          ctx.fillText(m.label, mx + 13, 8)
        }
      }
      ctx.restore()
    }

    // Edit cursor (dashed teal line, separate from red playhead)
    if (editCursorTicks != null) {
      const ecX = editCursorTicks * pixelsPerTick - scrollOffset
      if (ecX >= -2 && ecX <= w + 2) {
        ctx.strokeStyle = '#14B8A6'
        ctx.lineWidth = 1
        ctx.setLineDash([4, 3])
        ctx.beginPath()
        ctx.moveTo(ecX, RULER_HEIGHT)
        ctx.lineTo(ecX, h)
        ctx.stroke()
        ctx.setLineDash([])
        // Tiny caret in ruler
        ctx.fillStyle = '#14B8A6'
        ctx.beginPath()
        ctx.moveTo(ecX - 4, BAR_RULER_HEIGHT - 6)
        ctx.lineTo(ecX + 4, BAR_RULER_HEIGHT - 6)
        ctx.lineTo(ecX, BAR_RULER_HEIGHT)
        ctx.closePath()
        ctx.fill()
      }
    }

   }
   drawMainRef.current = draw
   draw()
   drawPlayheadRef.current()
  }, [tracks, rowIndex, rowCount, silent, colourPlace, layoutTempo, tempoEntries, tempoLaneShown, laneHot, laneScaleHeld, sampleRate, selectedClipId, selectedClipIds, looping, loopStart, loopEnd, trackHeight, horizontalZoom, snapValue, snapEnabled, editCursorTicks, markers, renamingMarker, clipToGroup, groupColors, punchEnabled, punchInTicks, punchOutTicks, verticalScroll, scrollX, followPlayhead, rulerUnits, timecodeFps, followMode, sections, meterSegments, peer])

  // The playhead: one element over the arrangement, moved with a
  // transform straight from the transport store. No React render, no
  // repaint: the compositor slides it, and the clips underneath are left
  // alone unless the view follows the playhead.
  const playheadRef = useRef<HTMLDivElement>(null)
  const followRef = useRef(followPlayhead)
  followRef.current = followPlayhead
  const viewRef = useRef({ pxPerTick: pixelsPerTick, scrollX, followMode })
  viewRef.current = { pxPerTick: pixelsPerTick, scrollX, followMode }
  useEffect(() => {
    // The view's width, kept by the ResizeObserver below: reading it on
    // every position forced a layout 30 times a second.
    let width = containerRef.current?.clientWidth ?? 0
    drawPlayheadRef.current = () => {
      const el = playheadRef.current
      const container = containerRef.current
      if (!el || !container) return
      const { positionSamples, playing } = useTransportStore.getState()
      const w = width
      const { pxPerTick, scrollX: sx, followMode: mode } = viewRef.current
      // Where the song is, on the bars it is drawn on: the same offset the
      // clips are drawn with, or the line and the clips part company.
      const px = tickAtPosition(positionSamples) * pxPerTick
      const offset = followRef.current ? followScroll(px, w, mode) : sx
      const x = Math.floor(px - offset)
      const show = (playing || positionSamples > 0) && x >= -6 && x <= w + 6
      el.style.display = show ? 'block' : 'none'
      if (show) el.style.transform = `translate3d(${x - PLAYHEAD_HALF}px, 0, 0)`
    }
    drawPlayheadRef.current()
    // A new size clears the canvas, so redraw it.
    const container = containerRef.current
    const ro = typeof ResizeObserver !== 'undefined' && container
      ? new ResizeObserver((entries) => {
        width = entries[entries.length - 1]?.contentRect.width ?? width
        drawMainRef.current()
        drawPlayheadRef.current()
      })
      : null
    if (ro && container) ro.observe(container)
    const unsubscribe = useTransportStore.subscribe((st, prev) => {
      if (st.positionSamples === prev.positionSamples && st.playing === prev.playing) return
      // Following scrolls the whole view, so the clips move too.
      if (followRef.current && st.positionSamples !== prev.positionSamples) drawMainRef.current()
      drawPlayheadRef.current()
    })
    return () => { unsubscribe(); ro?.disconnect() }
  }, [])

  function drawClip(
    ctx: CanvasRenderingContext2D,
    clip: ClipInfo,
    baseColor: string,
    trackY: number,
    scrollOffset: number,
    viewWidth: number,
    pxPerTick: number,
    lane: number = 0,
    laneCount: number = 1,
  ) {
    const clipX = clip.position_ticks * pxPerTick - scrollOffset
    const clipW = clip.length_ticks * pxPerTick
    if (clipX + clipW < 0 || clipX > viewWidth) return

    const pad = 1
    const laneHeight = trackHeight / Math.max(1, laneCount)
    const x = clipX
    const y = trackY + lane * laneHeight + pad
    const w = clipW
    const h = laneHeight - pad * 2
    const isSelected = clip.id === selectedClipId || selectedClipIds.has(clip.id)
    const color = clip.muted ? '#1a1a24' : baseColor
    // Clip chrome (rework proposal 2026-10-10): a readable title bar in a deeper
    // shade of the track colour with the name in Inter, a body tinted by the
    // track colour with a hairline border, and selection shown as an outline so
    // the colour identity stays. The waveform drawing below is unchanged.
    const radius = 3
    const headerH = h >= 34 ? 15 : Math.min(12, Math.max(9, Math.round(h * 0.34)))
    const shade = mixHex(color, '#000000', 0.28)
    const lum = hexLuma(color)

    ctx.beginPath()
    ctx.roundRect(x, y, w, h, radius)
    ctx.fillStyle = '#100f15'
    ctx.globalAlpha = clip.muted ? 0.7 : 0.95
    ctx.fill()
    ctx.fillStyle = color
    ctx.globalAlpha = clip.muted ? 0.05 : 0.17
    ctx.fill()
    ctx.globalAlpha = clip.muted ? 0.25 : 0.5
    ctx.strokeStyle = color
    ctx.lineWidth = 1
    ctx.beginPath()
    ctx.roundRect(x + 0.5, y + 0.5, w - 1, h - 1, radius)
    ctx.stroke()
    ctx.globalAlpha = 1.0

    ctx.fillStyle = clip.muted ? '#1d1d26' : shade
    ctx.beginPath()
    ctx.roundRect(x, y, w, headerH, [radius, radius, 0, 0])
    ctx.fill()

    if (headerH >= 9 && w >= 18) {
      const fontPx = headerH >= 15 ? 10.5 : 9
      ctx.fillStyle = clip.muted ? '#71717a' : lum > 0.62 ? 'rgba(10,10,12,0.92)' : 'rgba(255,255,255,0.95)'
      ctx.font = `600 ${fontPx}px Inter, 'Segoe UI', system-ui, sans-serif`
      ctx.textBaseline = 'middle'
      ctx.save()
      ctx.beginPath()
      ctx.rect(x + 3, y, w - 6, headerH)
      ctx.clip()
      ctx.fillText(clip.name, x + 6, y + headerH / 2 + 0.5)
      ctx.restore()
      ctx.textBaseline = 'alphabetic'
    }

    if (isSelected) {
      ctx.strokeStyle = '#ffffff'
      ctx.lineWidth = 1.5
      ctx.beginPath()
      ctx.roundRect(x + 0.75, y + 0.75, w - 1.5, h - 1.5, radius)
      ctx.stroke()
    }

    // A MIDI clip shows its notes, as FL shows a pattern in the playlist: a
    // thin bar per note, the clip's own lowest to highest note filling the
    // body, so a bass line and a lead read differently at a glance.
    if (clip.kind === 'midi' && clip.notePreview?.length && h - headerH > 6) {
      drawNotePreview(ctx, clip.notePreview, color, clip.muted, x, y + headerH, w, h - headerH, pxPerTick)
    }

    // Waveform — pick the best available tier: prefer the current one, else fallback
    // to any other tier we've already fetched so we render something during refetch.
    // Past WHOLE_TIER the buckets come in tiles (see wantTile), with the
    // whole-file waveform standing in for a tile still on its way.
    const tier = clipTier(clip, pxPerTick, layoutTempo)
    const tiled = tier > WHOLE_TIER
    let peaks = waveformData.get(`${clip.source_id}:${Math.min(tier, WHOLE_TIER)}`)
    if (!peaks) {
      for (const [k, v] of waveformData) {
        if (k.startsWith(`${clip.source_id}:`)) { peaks = v; break }
      }
    }
    if ((peaks?.length || tiled) && !clip.muted) {
      ctx.save()
      ctx.beginPath()
      ctx.rect(x + 1, y + headerH, w - 2, h - headerH - 1)
      ctx.clip()

      const waveArea = h - headerH - 2
      const midY = y + headerH + waveArea * 0.5
      const ampScale = waveArea * 0.46
      const whole = peaks ?? []
      const n = tiled ? tier : whole.length
      const peakAt = (j: number): WavePeak | undefined => {
        if (tiled) {
          const t = waveformTiles.get(`${clip.source_id}:${tier}:${Math.floor(j / TILE)}`)
          if (t) return t[j % TILE]
          return whole[Math.floor((j * whole.length) / n)]
        }
        return whole[j]
      }
      // Where bucket j of the file sits. With the file's length known, the
      // file is laid out at the speed it plays and from where the clip
      // starts in it, so what is drawn is what is heard. It used to be
      // the whole file squeezed into the clip, which only matched an
      // untrimmed clip exactly as long as its file.
      let bucketX = (j: number) => x + j * (w / n)
      let bucketAt = (px: number) => (px - x) / (w / n)
      let pxPerBucket = w / n
      if (clip.sourceFrames && clip.sourceRate) {
        const stretch = clip.timeFactor ?? (clip.stretchRatio > 0 ? clip.stretchRatio : 1)
        // While its left edge is dragged the audio stays where it plays:
        // the start is cut off, not moved, so draw from where it began.
        const trimming = dragRef.current
        const anchorTick =
          trimming && trimming.mode === 'resize-left' && trimming.clipId === clip.id && !clip.reversed
            ? trimming.originalPositionTicks
            : clip.position_ticks
        const anchorX = anchorTick * pxPerTick - scrollOffset
        // The file along the playlist as the engine plays it: over the
        // beats when the clip follows the tempo, in seconds from its start
        // when it keeps its speed (`clipTime.ts`).
        const layout = fileLayout({
          entries: tempoEntries, anchorTick, lengthTicks: clip.length_ticks, pxPerTick,
          sourceStartSec: (clip.sourceStart ?? 0) / clip.sourceRate, timeFactor: stretch,
          followBpm: clip.treatment?.followBpm, warped: (clip.warpMarkers?.length ?? 0) > 0,
        })
        const bucketSec = clip.sourceFrames / clip.sourceRate / n
        bucketX = (j: number) => anchorX + layout.xAt(j * bucketSec)
        bucketAt = (px: number) => layout.secAt(px - anchorX) / bucketSec
        pxPerBucket = bucketSec * layout.pxPerSec
      }
      // Reversed, the engine reads the clip's own stretch of the file from
      // its end back to its start, so that stretch is drawn mirrored inside
      // the clip. Mirroring the whole file instead drew a trimmed reversed
      // clip with audio it does not play.
      const backwards = clip.reversed
      const place = (bx: number) => (backwards ? 2 * x + w - bx - pxPerBucket : bx)

      // Only walk the buckets that fall inside the clip and the viewport.
      const left = Math.max(x, 0)
      const right = Math.min(x + w, viewWidth)
      const from = backwards ? 2 * x + w - right : left
      const to = backwards ? 2 * x + w - left : right
      const startJ = Math.max(0, Math.floor(bucketAt(from)) - 1)
      const endJ = Math.min(n, Math.ceil(bucketAt(to)) + 1)

      if (tiled) {
        for (let t = Math.floor(startJ / TILE); t <= Math.floor((endJ - 1) / TILE); t++) {
          wantTile(clip.source_id, tier, t)
        }
      }

      // One bucket per sample and two pixels or more apart: draw the signal
      // itself as a line, as FL does when zoomed in that far. A one-sample
      // bucket holds its sample as its max, or its min below zero, with
      // the other end at 0, so the sample is min + max.
      const sampleLine = n === clip.sourceFrames && pxPerBucket >= 2
      if (sampleLine && endJ > startJ) {
        const exactAt = (j: number): WavePeak | undefined =>
          tiled ? waveformTiles.get(`${clip.source_id}:${tier}:${Math.floor(j / TILE)}`)?.[j % TILE] : whole[j]
        ctx.lineWidth = 1.25
        ctx.lineJoin = 'round'
        let colour = ''
        let open = false
        const finish = () => { if (open) { ctx.strokeStyle = colour; ctx.stroke() } open = false }
        for (let j = startJ; j < endJ; j++) {
          const pk = exactAt(j)
          // A tile still on its way leaves a gap rather than a wrong line.
          if (!pk) { finish(); continue }
          const centre = bucketX(j + 0.5)
          const cx = backwards ? 2 * x + w - centre : centre
          const cy = midY - (pk[0] + pk[1]) * ampScale
          const c = rgbColour(pk[3], pk[4], pk[5])
          if (!open) {
            ctx.beginPath()
            ctx.moveTo(cx, cy)
            colour = c
            open = true
          } else {
            ctx.lineTo(cx, cy)
            // The colour moves slowly (it holds over 30 ms), so a new
            // stroke starts only where it changes.
            if (c !== colour) {
              ctx.strokeStyle = colour
              ctx.stroke()
              ctx.beginPath()
              ctx.moveTo(cx, cy)
              colour = c
            }
          }
        }
        finish()
      } else if (endJ > startJ) {
        // FL Studio's Multiband look: one waveform, each column in the
        // colour of what sounds there (the backend mixes it from a low, a
        // mid and a high band, peaks[j][3..5]), so a kick's tail is red and
        // its click purple-blue. Whole-pixel columns keep the outline crisp.
        const colW = Math.max(1, Math.ceil(pxPerBucket))
        for (let j = startJ; j < endJ; j++) {
          const bx = Math.floor(place(bucketX(j)))
          const pk = peakAt(j)
          if (!pk) continue
          ctx.fillStyle = rgbColour(pk[3], pk[4], pk[5])
          const top = midY - pk[1] * ampScale
          const bot = midY - pk[0] * ampScale
          ctx.fillRect(bx, top, colW, Math.max(1, bot - top))
        }
      }

      if (endJ > startJ) {
        // Faint centre baseline.
        ctx.globalAlpha = 0.18
        ctx.strokeStyle = 'rgba(255,255,255,0.5)'
        ctx.lineWidth = 0.5
        ctx.beginPath()
        ctx.moveTo(x + 1, midY)
        ctx.lineTo(x + w - 1, midY)
        ctx.stroke()
      }

      ctx.globalAlpha = 1.0
      ctx.restore()
    }

    // A clip that follows the tempo says how far the song stretches it
    // where it sits, as the mockup's loop across the build does.
    if (clip.kind === 'audio' && h - headerH > 24 && w > 90) {
      const label = stretchLabel(tempoEntries, clip.position_ticks, clip.length_ticks, clip.treatment?.followBpm, clip.timeFactor ?? 1)
      if (label) {
        ctx.save()
        ctx.beginPath()
        ctx.rect(x, y + headerH, w, h - headerH)
        ctx.clip()
        ctx.font = '9px Inter, ui-sans-serif, sans-serif'
        ctx.textBaseline = 'alphabetic'
        ctx.fillStyle = mixHex(color, '#ffffff', 0.6)
        ctx.fillText(label, x + 6, y + h - 5)
        ctx.restore()
      }
    }

    // Fade-in / fade-out overlays (triangular gradients)
    if (clip.fadeInTicks > 0 || clip.fadeOutTicks > 0) {
      ctx.save()
      ctx.beginPath()
      ctx.rect(x, y + headerH, w, h - headerH)
      ctx.clip()
      ctx.fillStyle = 'rgba(0,0,0,0.55)'
      if (clip.fadeInTicks > 0) {
        const fw = Math.min(w, clip.fadeInTicks * pxPerTick)
        ctx.beginPath()
        ctx.moveTo(x, y + headerH)
        ctx.lineTo(x + fw, y + headerH)
        ctx.lineTo(x, y + h)
        ctx.closePath()
        ctx.fill()
      }
      if (clip.fadeOutTicks > 0) {
        const fw = Math.min(w, clip.fadeOutTicks * pxPerTick)
        ctx.beginPath()
        ctx.moveTo(x + w, y + headerH)
        ctx.lineTo(x + w - fw, y + headerH)
        ctx.lineTo(x + w, y + h)
        ctx.closePath()
        ctx.fill()
      }
      ctx.restore()
    }

    // Reverse indicator
    if (clip.reversed) {
      ctx.fillStyle = '#EF4444'
      ctx.font = 'bold 8px Inter, ui-sans-serif, sans-serif'
      ctx.fillText('◄', x + w - 10, y + 10)
    }

    // No border — clips are borderless by design. Selection is shown by the
    // red header stripe above, not an outline.

    // Group indicator — tinted tab on top-left corner
    const gid = clipToGroup[clip.id]
    if (gid) {
      const gColor = groupColors[gid] || '#DC2626'
      ctx.save()
      ctx.fillStyle = gColor
      ctx.globalAlpha = 0.95
      const tabW = Math.min(18, Math.max(8, w - 4))
      ctx.beginPath()
      ctx.roundRect(x + 2, y + 2, tabW, 3, [2, 2, 0, 0])
      ctx.fill()
      ctx.restore()
    }
  }

  /** The name an automation clip carries in its header: the track whose
   *  knob it moves, and the knob, as FL names one ("909 Snare - Volume"). */
  function automationClipName(item: Extract<RowItem<TrackWithClips>, { kind: 'auto' }>): string {
    const clip = item.clip as AutomationClipInfo
    const target = clip.targetTrack ? useTrackStore.getState().tracksById[clip.targetTrack] : item.track
    return `${target?.name ?? item.track.name} - ${automationTargetLabel(clip.target)}`
  }

  /**
   * An automation clip on the grid, like any clip: a body in its colour,
   * its name in the header, and its curve with a dot on every point. It
   * used to be a strip of its own under its track, drawn at a scale of its
   * own that drifted from the bars, and it could not leave that strip.
   */
  function drawAutomationClip(
    ctx: CanvasRenderingContext2D,
    item: Extract<RowItem<TrackWithClips>, { kind: 'auto' }>,
    rowY: number,
    scrollOffset: number,
    viewWidth: number,
    pxPerTick: number,
  ) {
    const clip = item.clip as AutomationClipInfo
    const x = clip.startTick * pxPerTick - scrollOffset
    const w = Math.max(2, clip.lengthTicks * pxPerTick)
    if (x + w < 0 || x > viewWidth) return
    const color = argbHex(clip.colorArgb)
    const y = rowY + 1
    const h = trackHeight - 2
    const headerH = h >= 34 ? 15 : Math.min(12, Math.max(9, Math.round(h * 0.34)))
    const radius = 3
    ctx.beginPath()
    ctx.roundRect(x, y, w, h, radius)
    ctx.fillStyle = '#100f15'
    ctx.fill()
    ctx.fillStyle = color
    ctx.globalAlpha *= 0.22
    ctx.fill()
    ctx.globalAlpha /= 0.22
    ctx.fillStyle = mixHex(color, '#000000', 0.28)
    ctx.beginPath()
    ctx.roundRect(x, y, w, headerH, [radius, radius, 0, 0])
    ctx.fill()
    if (headerH >= 9 && w >= 18) {
      ctx.fillStyle = hexLuma(color) > 0.62 ? 'rgba(10,10,12,0.92)' : 'rgba(255,255,255,0.95)'
      ctx.font = `600 ${headerH >= 15 ? 10.5 : 9}px Inter, 'Segoe UI', system-ui, sans-serif`
      ctx.textBaseline = 'middle'
      ctx.save()
      ctx.beginPath()
      ctx.rect(x + 3, y, w - 6, headerH)
      ctx.clip()
      ctx.fillText(automationClipName(item), x + 6, y + headerH / 2 + 0.5)
      ctx.restore()
      ctx.textBaseline = 'alphabetic'
    }
    // The curve, in the body under the header.
    const top = y + headerH + 2
    const bottom = y + h - 3
    const at = (p: { tick: number; value: number }) => ({
      px: x + p.tick * pxPerTick,
      py: bottom - Math.max(0, Math.min(1, p.value)) * (bottom - top),
    })
    const pts = clip.points.map(at)
    if (bottom > top && pts.length > 0) {
      ctx.save()
      ctx.beginPath()
      ctx.rect(x, y, w, h)
      ctx.clip()
      ctx.strokeStyle = '#ffffff'
      ctx.globalAlpha *= 0.85
      ctx.lineWidth = 1.25
      ctx.beginPath()
      ctx.moveTo(x, pts[0].py)
      for (const p of pts) ctx.lineTo(p.px, p.py)
      ctx.lineTo(x + w, pts[pts.length - 1].py)
      ctx.stroke()
      ctx.fillStyle = '#ffffff'
      for (const p of pts) {
        ctx.beginPath()
        ctx.arc(p.px, p.py, 2.2, 0, Math.PI * 2)
        ctx.fill()
      }
      ctx.restore()
    }
  }

  /** The playlist row under a pointer position (scrolled), or -1 over the
   *  ruler. */
  const rowAt = useCallback((mouseY: number) => {
    return rowAtY(mouseY, RULER_HEIGHT, trackHeight, verticalScroll)
  }, [trackHeight, verticalScroll])

  /** What is under the pointer, the one drawn on top: a clip or an
   *  automation clip, and which part of it. Only the row under the pointer
   *  and what reaches the pointer's bar on it are looked at. */
  const itemHit = useCallback((mouseX: number, mouseY: number, scrollOffset: number): {
    item: RowItem<TrackWithClips>
    edge: 'body' | 'left' | 'right' | 'fade-in' | 'fade-out'
    /** For an automation clip, the point under the pointer. */
    point?: number
  } | null => {
    const row = rowAt(mouseY)
    if (row < 0 || pixelsPerTick <= 0) return null
    const rowTop = RULER_HEIGHT + row * trackHeight - verticalScroll
    const localY = mouseY - rowTop
    if (localY < 2 || localY > trackHeight - 2) return null
    const tick = (mouseX + scrollOffset) / pixelsPerTick
    const item = itemAt(rowIndex, row, tick)
    if (!item) return null
    const clipX = item.start * pixelsPerTick - scrollOffset
    const clipW = (item.end - item.start) * pixelsPerTick
    if (item.kind === 'auto') {
      const clip = item.clip as AutomationClipInfo
      const h = trackHeight - 2
      const headerH = h >= 34 ? 15 : Math.min(12, Math.max(9, Math.round(h * 0.34)))
      const top = rowTop + 1 + headerH + 2
      const bottom = rowTop + trackHeight - 4
      const point = clip.points.findIndex((p) => {
        const px = clipX + p.tick * pixelsPerTick
        const py = bottom - Math.max(0, Math.min(1, p.value)) * (bottom - top)
        return Math.abs(px - mouseX) <= 5 && Math.abs(py - mouseY) <= 5
      })
      if (point >= 0) return { item, edge: 'body', point }
      return { item, edge: mouseX > clipX + clipW - RESIZE_HANDLE_PX ? 'right' : 'body' }
    }
    const withinFadeBand = localY - 2 >= HEADER_H && localY - 2 <= HEADER_H + FADE_HANDLE_PX && clipW > FADE_HANDLE_PX * 3
    // Fade handles take priority over resize edges within the narrow top band.
    if (withinFadeBand) {
      if (mouseX <= clipX + RESIZE_HANDLE_PX + FADE_HANDLE_PX) return { item, edge: 'fade-in' }
      if (mouseX >= clipX + clipW - RESIZE_HANDLE_PX - FADE_HANDLE_PX) return { item, edge: 'fade-out' }
    }
    if (mouseX > clipX + clipW - RESIZE_HANDLE_PX) return { item, edge: 'right' }
    if (mouseX < clipX + RESIZE_HANDLE_PX) return { item, edge: 'left' }
    return { item, edge: 'body' }
  }, [rowIndex, rowAt, pixelsPerTick, trackHeight, verticalScroll])

  /**
   * Drop one copy of the armed picker clip in a grid slot.
   *
   * Used by the paint tool, which crosses many slots in one drag. Each slot
   * is recorded before its placement runs, because the pointer keeps moving
   * while the IPC is in flight, and placements are serialised through one
   * chain: a placement is a duplicate followed by a move, and two of those
   * in parallel let one slot's move land on the other slot's copy.
   */
  const paintClipAt = useCallback((drag: DragState, row: number, tick: number) => {
    const selection = usePickerStore.getState().selection
    if (selection?.kind !== 'audioClip' || row < 0) return
    const slot = `${row}:${tick}`
    if (!drag.paintedSlots || drag.paintedSlots.has(slot)) return

    const length = drag.paintStepTicks ?? PPQ
    // Anything already on that stretch of the row, whichever channel.
    const onRow = itemsInView(rowIndex, row, tick, tick + length)
      .filter((i) => i.kind === 'clip')
      .map((i) => i.clip as ClipInfo)
    if (paintSlotOccupied(onRow, tick, length)) return

    drag.paintedSlots.add(slot)
    drag.paintChain = (drag.paintChain ?? Promise.resolve())
      .then(() => useTrackStore
        .getState()
        // No per-clip undo entry: the gesture pushes one when it ends.
        .placeClipCopy(selection.trackId, selection.clipId, row, tick, null)
        .then(() => undefined))
      .catch(err => console.error('paint placeClipCopy failed', err))
  }, [rowIndex])

  /** The clip (audio or MIDI) under the pointer and which part of it. */
  const hitTest = useCallback((mouseX: number, mouseY: number, scrollOffset: number): {
    clip: ClipInfo, trackId: string, row: number, edge: 'body' | 'left' | 'right' | 'fade-in' | 'fade-out'
  } | null => {
    const hit = itemHit(mouseX, mouseY, scrollOffset)
    if (!hit || hit.item.kind !== 'clip') return null
    return { clip: hit.item.clip as ClipInfo, trackId: hit.item.track.id, row: hit.item.row, edge: hit.edge }
  }, [itemHit])

  const getScrollOffset = useCallback(() => {
    if (!followPlayhead) return scrollX
    const { positionSamples } = useTransportStore.getState()
    const container = containerRef.current
    const w = container ? container.getBoundingClientRect().width : 800
    return followScroll(tickAtPosition(positionSamples) * pixelsPerTick, w, followMode)
  }, [pixelsPerTick, followPlayhead, followMode, scrollX]) // eslint-disable-line react-hooks/exhaustive-deps

  // View > Zoom to selection: the selected clips fill the width, with a
  // little room either side, and the view stops following the playhead so
  // it stays on them.
  useEffect(() => {
    const zoomToSelection = () => {
      const { selectedClipIds: ids } = useTrackStore.getState()
      const picked = audioTracks.flatMap(t => t.clips.filter(c => ids.has(c.id)))
      const container = containerRef.current
      if (picked.length === 0 || !container || layoutTempo <= 0) return
      const start = Math.min(...picked.map(c => c.position_ticks))
      const end = Math.max(...picked.map(c => c.position_ticks + c.length_ticks))
      const w = container.getBoundingClientRect().width
      const secs = ((end - start) / PPQ) * (60 / layoutTempo)
      if (secs <= 0 || w <= 0) return
      const store = useTransportStore.getState()
      store.setHorizontalZoom((w * 0.9) / (secs * PIXELS_PER_SECOND_BASE))
      const pps = PIXELS_PER_SECOND_BASE * useTransportStore.getState().horizontalZoom
      setFollowPlayhead(false)
      setScrollX(Math.max(0, (start / PPQ) * (60 / layoutTempo) * pps - w * 0.05))
    }
    window.addEventListener('daw:zoomToSelection', zoomToSelection)
    return () => window.removeEventListener('daw:zoomToSelection', zoomToSelection)
  }, [audioTracks, layoutTempo, setFollowPlayhead])

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    // Middle button held: slide the playlist left and right, as in FL.
    // Followed on the window, so the drag keeps going past the edge.
    if (e.button === 1) {
      e.preventDefault()
      const startX = e.clientX
      const startScroll = getScrollOffset()
      if (followPlayhead) setFollowPlayhead(false)
      const previousCursor = document.body.style.cursor
      document.body.style.cursor = 'grabbing'
      const move = (ev: MouseEvent) => {
        setScrollX(Math.max(0, startScroll - (ev.clientX - startX)))
      }
      const up = (ev: MouseEvent) => {
        if (ev.button !== 1) return
        window.removeEventListener('mousemove', move)
        window.removeEventListener('mouseup', up)
        document.body.style.cursor = previousCursor
      }
      window.addEventListener('mousemove', move)
      window.addEventListener('mouseup', up)
      return
    }
    if (e.button === 2) {
      // Right-mouse-button drag → 2D pan. Stash the start position so
      // pointermove can compute deltas and onContextMenu can decide
      // whether to suppress the menu based on whether we actually
      // panned (moved > 4px) or just clicked.
      rightPanRef.current = {
        startY: e.clientY,
        startVerticalScroll: verticalScroll,
        moved: false,
      }
      return
    }
    setContextMenu(null)
    showTipLater(null)
    const rect = (e.target as HTMLElement).getBoundingClientRect()
    const mouseX = e.clientX - rect.left
    const mouseY = e.clientY - rect.top
    const scrollOffset = getScrollOffset()

    // Where you are working, for the other person in the room. Does
    // nothing when there is no room.
    {
      const tick = (mouseX + scrollOffset) / pixelsPerTick
      const row = mouseY > RULER_HEIGHT && trackHeight > 0
        ? Math.floor((mouseY - RULER_HEIGHT + verticalScroll) / trackHeight)
        : null
      useCollabStore.getState().share(tick, row, 'arrangement')
    }

    // Clicking the ruler strip seeks the transport (and enables scrubbing while held).
    if (mouseY < RULER_HEIGHT) {
      // Ctrl/⌘ + drag in the ruler → define a loop region. Pick this
      // branch BEFORE the marker-hit + scrub branches so a held modifier
      // wins over both. Anchor tick is snapped at mousedown; mousemove
      // updates the live edge; mouseup commits via setLoop.
      if (e.ctrlKey || e.metaKey) {
        const anchorTick = Math.max(0, Math.round((mouseX + scrollOffset) / pixelsPerTick))
        const snappedAnchor = applySnap(anchorTick)
        dragRef.current = {
          mode: 'loop-range', clipId: '', trackId: '',
          startMouseX: mouseX, startMouseY: mouseY, currentMouseX: mouseX, currentMouseY: mouseY,
          originalPositionTicks: snappedAnchor, originalLengthTicks: 0,
          originalFadeInTicks: 0, originalFadeOutTicks: 0,
        }
        forceRender(n => n + 1)
        return
      }
      // Marker flag hit test — flag sits between x and x+10, top half of the ruler.
      const markerHit = markers.find(m => {
        const mx = m.tick * pixelsPerTick - scrollOffset
        return mouseX >= mx - 1 && mouseX <= mx + 11 && mouseY <= RULER_HEIGHT - 4
      })
      if (markerHit) {
        setPosition(positionAtTick(markerHit.tick))
        return
      }
      setPosition(positionAtTick(Math.max(0, (mouseX + scrollOffset) / pixelsPerTick)))
      dragRef.current = {
        mode: 'scrub', clipId: '', trackId: '',
        startMouseX: mouseX, startMouseY: mouseY, currentMouseX: mouseX, currentMouseY: mouseY,
        originalPositionTicks: 0, originalLengthTicks: 0,
        originalFadeInTicks: 0, originalFadeOutTicks: 0,
        // Freeze the offset for the whole gesture — see DragState doc.
        scrubScrollOffset: scrollOffset,
      }
      return
    }

    // Tool-mode branch — selected playlist tool wins over the default
    // "draw + hold-to-marquee" behaviour. Most tools are a one-shot action
    // on the clip under the cursor; `paint`, `slip` and `select` start their
    // own drag, and `zoom` keeps wheel semantics.
    const currentTool = usePlaylistToolStore.getState().tool
    if (currentTool === 'paint') {
      // Drag across the playlist and the armed clip lands in every grid slot
      // the pointer crosses. Until now this tool fell through to the default
      // single paste, so the toolbar offered a paint brush that painted once.
      const selection = usePickerStore.getState().selection
      if (selection?.kind !== 'audioClip') {
        onSetHint?.(t('Paint tool: pick a clip in the picker panel first, then drag across the playlist'))
        return
      }
      const source = useTrackStore.getState().tracks
        .find(t => t.id === selection.trackId)?.clips
        ?.find(c => c.id === selection.clipId)
      // The grid decides the spacing, and with snap off the clip's own
      // length does, so painted copies sit end to end instead of on top of
      // each other.
      const step = snapTicks > 0 ? snapTicks : Math.max(1, source?.length_ticks ?? PPQ)
      // Copies land on the row under the pointer, each on the channel of
      // the clip they copy, so they sound as that clip does.
      const targetRow = rowAt(mouseY)
      if (targetRow < 0) return
      const tick = paintSlotTick(mouseX, scrollOffset, pixelsPerTick, step)
      // One drag is one undo, however many clips it lays down.
      useTrackStore.getState().beginHistoryGroup()
      dragRef.current = {
        mode: 'paint', clipId: '', trackId: selection.trackId,
        startMouseX: mouseX, startMouseY: mouseY,
        currentMouseX: mouseX, currentMouseY: mouseY,
        originalPositionTicks: tick, originalLengthTicks: 0,
        originalFadeInTicks: 0, originalFadeOutTicks: 0,
        paintedSlots: new Set<string>(), paintStepTicks: step,
      }
      paintClipAt(dragRef.current, targetRow, tick)
      return
    }
    if (currentTool === 'slice' || currentTool === 'delete') {
      const hit = hitTest(mouseX, mouseY, scrollOffset)
      if (!hit) return
      if (currentTool === 'slice') {
        // Split at the clicked tick, snapped. splitClip returns the new
        // clip id but we ignore it — both halves stay on the track.
        const sliceTick = applySnap(
          Math.max(0, Math.round((mouseX + scrollOffset) / pixelsPerTick)),
        )
        useTrackStore
          .getState()
          .splitClip(hit.trackId, hit.clip.id, sliceTick)
          .catch((err) => console.error('splitClip failed', err))
      } else {
        useTrackStore
          .getState()
          .deleteClip(hit.trackId, hit.clip.id)
          .catch((err) => console.error('deleteClip failed', err))
      }
      return
    }
    if (currentTool === 'mute') {
      // Was falling through to the default paste, so the tool silently did
      // nothing. Pattern clips could not be muted at all until the flag was
      // added to MidiClip.
      const hit = hitTest(mouseX, mouseY, scrollOffset)
      if (!hit) return
      useTrackStore
        .getState()
        .setClipMuted(hit.trackId, hit.clip.id, !hit.clip.muted)
        .catch((err) => console.error('setClipMuted failed', err))
      return
    }
    if (currentTool === 'slip') {
      // Drag inside a clip slides the audio within it while the clip stays
      // where it is. Tracked as its own drag mode; the shift is applied on
      // release so one gesture is one undo step.
      const hit = hitTest(mouseX, mouseY, scrollOffset)
      if (!hit) return
      // The whole gesture is one undo step, however many moves it sends.
      useTrackStore.getState().beginHistoryGroup()
      dragRef.current = {
        slipSamples: 0,
        mode: 'slip', clipId: hit.clip.id, trackId: hit.trackId,
        startMouseX: mouseX, startMouseY: mouseY,
        currentMouseX: mouseX, currentMouseY: mouseY,
        originalPositionTicks: hit.clip.position_ticks,
        originalLengthTicks: hit.clip.length_ticks,
        originalFadeInTicks: 0, originalFadeOutTicks: 0,
      }
      forceRender(n => n + 1)
      return
    }
    if (currentTool === 'zoom') {
      // Click in the playlist body → zoom in (or out with Alt). The
      // zoom is centred on the wheel position so the bar under the
      // cursor stays roughly fixed.
      const factor = e.altKey ? 1 / 1.5 : 1.5
      setHorizontalZoom(horizontalZoom * factor)
      return
    }
    if (currentTool === 'select') {
      // Pure rubber-band — skip the picker-paste fall-through and
      // start the marquee immediately without the 4px threshold.
      if (!(e.ctrlKey || e.metaKey)) clearSelection()
      dragRef.current = {
        mode: 'rubber', clipId: '', trackId: '',
        startMouseX: mouseX, startMouseY: mouseY,
        currentMouseX: mouseX, currentMouseY: mouseY,
        originalPositionTicks: 0, originalLengthTicks: 0,
        originalFadeInTicks: 0, originalFadeOutTicks: 0,
      }
      forceRender(n => n + 1)
      return
    }
    // An automation clip: grab a point to move it, the right edge to
    // change its length, anywhere else to move it along the playlist and
    // up or down the rows. It keeps moving the same knob wherever it sits.
    const top = itemHit(mouseX, mouseY, scrollOffset)
    if (top && top.item.kind === 'auto') {
      const clip = top.item.clip as AutomationClipInfo
      const point = top.point !== undefined ? clip.points[top.point] : undefined
      dragRef.current = {
        mode: top.point !== undefined ? 'move' : top.edge === 'right' ? 'resize-right' : 'move',
        clipId: clip.id,
        trackId: top.item.track.id,
        startMouseX: mouseX, startMouseY: mouseY, currentMouseX: mouseX, currentMouseY: mouseY,
        originalPositionTicks: clip.startTick,
        originalLengthTicks: clip.lengthTicks,
        originalFadeInTicks: 0, originalFadeOutTicks: 0,
        originalRow: top.item.row,
        startRow: top.item.row,
        automation: point
          ? { pointIndex: top.point, pointTick: point.tick, pointValue: point.value }
          : {},
      }
      return
    }
    const hit = hitTest(mouseX, mouseY, scrollOffset)
    if (hit) {
      const gid = clipToGroup[hit.clip.id]
      const groupMemberIds = gid
        ? Object.keys(clipToGroup).filter(k => clipToGroup[k] === gid)
        : [hit.clip.id]
      if (e.ctrlKey || e.metaKey) {
        toggleClipSelection(hit.clip.id)
      } else if (!selectedClipIds.has(hit.clip.id)) {
        if (gid && groupMemberIds.length > 1) {
          useTrackStore.setState(() => {
            const next = new Set<string>(groupMemberIds)
            return { selectedClipIds: next, selectedClipId: hit.clip.id, selectedTrackId: hit.trackId }
          })
        } else {
          selectClip(hit.clip.id, hit.trackId)
        }
      } else {
        selectClip(hit.clip.id, hit.trackId)
      }
      const mode: DragMode =
        hit.edge === 'right' ? 'resize-right' :
        hit.edge === 'left' ? 'resize-left' :
        hit.edge === 'fade-in' ? 'fade-in' :
        hit.edge === 'fade-out' ? 'fade-out' :
        'move'
      let groupMoveOriginals: { clipId: string; trackId: string; origPos: number; origRow: number }[] | undefined
      if (mode === 'move' && gid && groupMemberIds.length > 1) {
        groupMoveOriginals = []
        for (const t of tracks) {
          for (const c of t.clips) {
            if (c.id === hit.clip.id) continue
            if (groupMemberIds.includes(c.id)) {
              groupMoveOriginals.push({
                clipId: c.id, trackId: t.id, origPos: c.position_ticks, origRow: rowIndex.rowOf.get(c.id) ?? 0,
              })
            }
          }
        }
      }
      dragRef.current = {
        mode,
        clipId: hit.clip.id,
        trackId: hit.trackId,
        startMouseX: mouseX,
        startMouseY: mouseY,
        currentMouseX: mouseX,
        currentMouseY: mouseY,
        originalPositionTicks: hit.clip.position_ticks,
        originalLengthTicks: hit.clip.length_ticks,
        originalFadeInTicks: hit.clip.fadeInTicks,
        originalFadeOutTicks: hit.clip.fadeOutTicks,
        groupMoveOriginals,
        originalRow: hit.row,
        startRow: rowAt(mouseY),
      }
    } else {
      // FL Studio place-mode: if the picker has an audio clip armed,
      // left-click in a track row places a copy at the clicked tick.
      // The rubber-band fallback only runs when no picker selection
      // is active (so existing select-multiple workflow is untouched).
      const pickerSel = usePickerStore.getState().selection
      // The copy goes on the row under the pointer, on the channel of the
      // clip it copies: it sounds as that clip does on any row.
      const targetRow = rowAt(mouseY)
      if (pickerSel?.kind === 'audioClip' && targetRow >= 0) {
        const tickAt = applySnap(
          Math.max(0, Math.round((mouseX + scrollOffset) / pixelsPerTick)),
        )
        useTrackStore
          .getState()
          .placeClipCopy(pickerSel.trackId, pickerSel.clipId, targetRow, tickAt)
          .catch(err => console.error('placeClipCopy failed', err))
        return
      }
      if (!(e.ctrlKey || e.metaKey)) clearSelection()
      // A click on a row picks it, as a click on its name does: F2 renames
      // it.
      if (targetRow >= 0) usePlaylistRowsStore.getState().selectRow(targetRow)
      // Don't commit anything yet — defer the decision until we know if the
      // user is doing a simple click (paint/no-op) or a drag (marquee).
      // The edit cursor and the rubber band both only kick in once we cross
      // RUBBER_PROMOTE_THRESHOLD_PX in handleMouseMove. This kills the
      // unwanted "teal edit-cursor line flashes on every click" UX.
      const tickAt = Math.max(0, Math.round((mouseX + scrollOffset) / pixelsPerTick))
      dragRef.current = {
        mode: 'pending-empty', clipId: '', trackId: '',
        startMouseX: mouseX, startMouseY: mouseY, currentMouseX: mouseX, currentMouseY: mouseY,
        originalPositionTicks: applySnap(tickAt), originalLengthTicks: 0,
        originalFadeInTicks: 0, originalFadeOutTicks: 0,
      }
      // No forceRender — pending state is invisible until promoted.
    }
  }, [hitTest, itemHit, rowIndex, selectClip, toggleClipSelection, clearSelection, selectedClipIds, getScrollOffset, PIXELS_PER_SECOND, sampleRate, setPosition, pixelsPerTick, setEditCursor, snapTicks, clipToGroup, tracks, markers, bpm, horizontalZoom, setHorizontalZoom, rowAt, paintClipAt, onSetHint, trackHeight, verticalScroll, followPlayhead, setFollowPlayhead])

  // The note shown over a clip the pointer rests on (mockup M1): what it
  // is, the channel and insert it plays through, and its row.
  const [tip, setTip] = useState<{ x: number; y: number; title: string; kind: string; through: string; row: string } | null>(null)
  const tipTimer = useRef<number | null>(null)
  const tipFor = useRef<string | null>(null)
  const showTipLater = useCallback((at: { item: RowItem<TrackWithClips>; x: number; y: number } | null) => {
    const id = at ? at.item.clip.id : null
    if (id === tipFor.current) return
    tipFor.current = id
    if (tipTimer.current != null) window.clearTimeout(tipTimer.current)
    tipTimer.current = null
    setTip(null)
    if (!at) return
    tipTimer.current = window.setTimeout(() => {
      const { item } = at
      const byId = useTrackStore.getState().tracksById
      const channel = item.track as TrackWithClips
      const row = `On row ${item.row + 1}. Drag it to another row: it sounds the same.`
      if (item.kind === 'auto') {
        const clip = item.clip as AutomationClipInfo
        const target = clip.targetTrack ? byId[clip.targetTrack] : channel
        setTip({
          x: at.x, y: at.y, title: automationClipName(item), kind: 'automation clip',
          through: `Moves ${automationTargetLabel(clip.target).toLowerCase()} of ${target?.name ?? channel.name}.`, row,
        })
        return
      }
      const clip = item.clip as ClipInfo
      setTip({
        x: at.x, y: at.y, title: clip.name, kind: clip.kind === 'midi' ? 'MIDI clip' : 'audio clip',
        through: `Plays through ${channel.name} into ${insertOf(channel, byId)}.`, row,
      })
    }, 600)
  }, [])
  useEffect(() => () => { if (tipTimer.current != null) window.clearTimeout(tipTimer.current) }, [])

  const handleMouseMove = useCallback((e: React.MouseEvent) => {
    // Right-mouse-button pan takes priority over any other drag mode.
    // It only moves vertically here — horizontal "pan" in this codebase
    // is derived from playhead position via getScrollOffset, not a
    // user-settable offset, so right-drag-Y is the meaningful axis.
    const rp = rightPanRef.current
    if (rp) {
      const dy = e.clientY - rp.startY
      if (Math.abs(dy) >= 4) rp.moved = true
      const container = containerRef.current
      if (container) {
        const viewportH = container.clientHeight - RULER_HEIGHT
          const maxScroll = Math.max(0, contentH - viewportH)
        const next = Math.max(0, Math.min(maxScroll, rp.startVerticalScroll - dy))
        setVerticalScroll(next)
      }
      return
    }

    const drag = dragRef.current
    if (!drag || drag.mode === 'none') {
      const rect = (e.target as HTMLElement).getBoundingClientRect()
      const mouseX = e.clientX - rect.left
      const mouseY = e.clientY - rect.top
      const top = itemHit(mouseX, mouseY, getScrollOffset())
      const hit = top && top.item.kind === 'clip'
        ? { clip: top.item.clip as ClipInfo, edge: top.edge }
        : null
      // What a clip is, where its sound goes and that its row does not
      // change it, shown once the pointer rests on it.
      showTipLater(top ? { item: top.item, x: e.clientX, y: e.clientY } : null)
      const canvas = canvasRef.current
      if (canvas) {
        if (mouseY < RULER_HEIGHT) {
          canvas.style.cursor = 'pointer'
        } else if (top && top.item.kind === 'auto') {
          canvas.style.cursor = top.point !== undefined ? 'move' : top.edge === 'right' ? 'ew-resize' : 'grab'
        } else if (hit) {
          if (hit.edge === 'right' || hit.edge === 'left') canvas.style.cursor = 'ew-resize'
          else if (hit.edge === 'fade-in' || hit.edge === 'fade-out') canvas.style.cursor = 'ns-resize'
          else canvas.style.cursor = 'grab'
        } else {
          canvas.style.cursor = 'default'
        }
      }
      // Hint bar at bottom of playlist — describe whatever is under the cursor.
      if (onSetHint) {
        if (mouseY < RULER_HEIGHT) {
          onSetHint(t('Timeline ruler — click to place playhead'))
        } else if (hit) {
          const tip = hit.edge === 'right' || hit.edge === 'left'
            ? t('drag to resize · alt to bypass snap')
            : hit.edge === 'fade-in'
              ? t('drag to set fade-in length')
              : hit.edge === 'fade-out'
                ? t('drag to set fade-out length')
                : t('drag to move · alt to bypass snap')
          onSetHint(`${hit.clip.name} · ${tip}`)
        } else {
          onSetHint('')
        }
      }
      return
    }

    const rect = (e.target as HTMLElement).getBoundingClientRect()
    const mouseX = e.clientX - rect.left
    const mouseY = e.clientY - rect.top
    drag.currentMouseX = mouseX
    drag.currentMouseY = mouseY
    const dx = mouseX - drag.startMouseX
    const dTicks = Math.round(dx / pixelsPerTick)
    // Magnetic snap: within 8 px of another clip's edge or a marker, a
    // moved or resized edge sticks to it; further away the grid decides.
    // Alt held leaves both out, as in FL Studio, for placing a clip by ear.
    const reach = 8 / pixelsPerTick
    const edgeTargets = (): number[] => {
      if (!drag.snapTargets) {
        const moving = new Set([drag.clipId, ...(drag.groupMoveOriginals ?? []).map(m => m.clipId)])
        const targets: number[] = []
        for (const t of audioTracks) {
          for (const c of t.clips) {
            if (moving.has(c.id)) continue
            targets.push(c.position_ticks, c.position_ticks + c.length_ticks)
          }
        }
        for (const m of markers) targets.push(m.tick)
        drag.snapTargets = targets
      }
      return drag.snapTargets
    }

    const minLen = snapTicks > 0 ? snapTicks : PPQ

    // Pending empty-area press: only promote to a real rubber-band selection
    // once the pointer has traveled past the threshold. Up to that point the
    // press is a candidate "click" — released without crossing the threshold
    // it's a no-op (handled in handleMouseUp). Crucially the promotion does
    // NOT set the edit cursor: the user's spec is "left-click is always
    // paste, hold-and-drag is select, neither one should ever flash a
    // cursor line". The edit cursor (paste origin) is only moved via
    // ruler clicks or future right-click context-menu actions.
    if (drag.mode === 'pending-empty') {
      const dy = mouseY - drag.startMouseY
      if (Math.sqrt(dx * dx + dy * dy) < RUBBER_PROMOTE_THRESHOLD_PX) return
      drag.mode = 'rubber'
      forceRender(n => n + 1)
      return
    }

    // Loop-range drag: update the live edge tick. Render the new range
    // via a forceRender — the canvas pass reads loopStart/loopEnd from
    // the transport store so we set those provisionally during the drag.
    if (drag.mode === 'loop-range') {
      const scrollOff = getScrollOffset()
      const liveTick = Math.max(0, Math.round((mouseX + scrollOff) / pixelsPerTick))
      const snappedLive = applySnap(liveTick)
      const anchor = drag.originalPositionTicks
      const a = Math.min(anchor, snappedLive)
      const b = Math.max(anchor, snappedLive)
      // Convert tick → samples through the tempo map, the same path the
      // canvas uses to draw the loop region, so the visual preview matches
      // whatever the engine will eventually loop.
      // Live preview only — don't IPC on every mousemove. The commit
      // happens once on mouseup via transport.setLoop.
      useTransportStore.setState({
        loopStart: positionAtTick(a),
        loopEnd: positionAtTick(b),
      })
      forceRender(n => n + 1)
      return
    }
    if (drag.mode === 'scrub') {
      // Use the offset frozen at scrub start, NOT the live auto-follow
      // offset — otherwise playback advancing the playhead would make the
      // mapped tick run away forward under a near-stationary cursor.
      const scrollOffset = drag.scrubScrollOffset ?? getScrollOffset()
      setPosition(positionAtTick(Math.max(0, (mouseX + scrollOffset) / pixelsPerTick)))
      return
    }

    if (drag.mode === 'rubber') {
      forceRender(n => n + 1)
      return
    }

    if (drag.mode === 'paint') {
      // One copy per slot crossed, on whichever row the pointer is over,
      // so a diagonal drag paints down the rows as well as along the bar.
      const step = drag.paintStepTicks ?? PPQ
      const row = rowAt(mouseY)
      if (row < 0) return
      const tick = paintSlotTick(mouseX, getScrollOffset(), pixelsPerTick, step)
      paintClipAt(drag, row, tick)
      return
    }

    if (drag.automation) {
      // An automation clip: shown moving while held, sent on release.
      const clip = useTrackStore.getState().tracksById[drag.trackId]?.automationClips?.find(c => c.id === drag.clipId)
      if (!clip) return
      const a = drag.automation
      let next: AutomationClipInfo
      if (a.pointIndex !== undefined) {
        // A point: along the clip (snapped) and up or down its body.
        const rowTop = RULER_HEIGHT + (drag.originalRow ?? 0) * trackHeight - verticalScroll
        const h = trackHeight - 2
        const headerH = h >= 34 ? 15 : Math.min(12, Math.max(9, Math.round(h * 0.34)))
        const top = rowTop + 1 + headerH + 2
        const bottom = rowTop + trackHeight - 4
        const raw = (a.pointTick ?? 0) + dTicks
        const tick = Math.max(0, Math.min(clip.lengthTicks, e.altKey ? raw : applySnap(raw)))
        const value = bottom > top ? Math.max(0, Math.min(1, (bottom - mouseY) / (bottom - top))) : (a.pointValue ?? 0.5)
        drag.lastPositionTicks = tick
        drag.fadesSent = [tick, value]
        const points = clip.points.map((p, i) => (i === a.pointIndex ? { ...p, tick, value } : p))
        next = { ...clip, points }
      } else if (drag.mode === 'resize-right') {
        const len = Math.max(minLen, drag.originalLengthTicks + dTicks)
        drag.lastLengthTicks = e.altKey ? len : Math.max(minLen, applySnap(len))
        next = { ...clip, lengthTicks: drag.lastLengthTicks }
      } else {
        const raw = Math.max(0, drag.originalPositionTicks + dTicks)
        drag.lastPositionTicks = e.altKey ? raw : applySnap(raw)
        const row = rowAt(mouseY)
        drag.lastRowDelta = row >= 0 ? row - (drag.startRow ?? row) : (drag.lastRowDelta ?? 0)
        next = {
          ...clip,
          startTick: drag.lastPositionTicks,
          row: Math.max(0, (drag.originalRow ?? 0) + (drag.lastRowDelta ?? 0)),
        }
      }
      useTrackStore.setState(st => {
        const tracks = st.tracks.map(t => t.id !== drag.trackId ? t : {
          ...t,
          automationClips: t.automationClips.map(c => (c.id === clip.id ? next : c)),
        })
        const tracksById = { ...st.tracksById }
        const changed = tracks.find(t => t.id === drag.trackId)
        if (changed) tracksById[changed.id] = changed
        return { tracks, tracksById }
      })
      return
    }

    if (drag.mode === 'slip') {
      // Dragging right pulls later audio into the clip, so the source offset
      // moves forward by the dragged distance, in samples as the offset into
      // a sample is. Only the part not sent yet goes: sending the whole
      // distance on every move added it again each time.
      // The time those ticks take where the clip is, through the tempo map,
      // not at the tempo the transport shows somewhere else in the song.
      const from = drag.originalPositionTicks
      const total = positionAtTick(Math.max(0, from + dTicks)) - positionAtTick(from)
      const step = total - (drag.slipSamples ?? 0)
      if (step !== 0) {
        drag.slipSamples = total
        drag.paintChain = (drag.paintChain ?? Promise.resolve())
          .then(() => useTrackStore.getState().slipClip(drag.trackId, drag.clipId, step))
          .catch((err) => console.error('slipClip failed', err))
      }
      return
    }

    if (drag.mode === 'fade-in' || drag.mode === 'fade-out') {
      // Dragging left shortens fade-out from the right edge, so invert dx.
      const fades: [number, number] = drag.mode === 'fade-in'
        ? [Math.max(0, Math.min(drag.originalLengthTicks, drag.originalFadeInTicks + dTicks)), drag.originalFadeOutTicks]
        : [drag.originalFadeInTicks, Math.max(0, Math.min(drag.originalLengthTicks, drag.originalFadeOutTicks - dTicks))]
      const last = drag.fadesSent ?? [drag.originalFadeInTicks, drag.originalFadeOutTicks]
      if (fades[0] === last[0] && fades[1] === last[1]) return
      // Heard while dragging, and one undo step for the drag: every move
      // used to be a step of its own, dozens for one fade.
      drag.fadesSent = fades
      openDragStep(drag)
      const { trackId, clipId } = drag
      drag.paintChain = (drag.paintChain ?? Promise.resolve())
        .then(() => setClipFades(trackId, clipId, fades[0], fades[1]))
        .catch((err) => console.error('setClipFades failed', err))
      return
    }
    if (drag.mode === 'move') {
      const raw = Math.max(0, drag.originalPositionTicks + dTicks)
      const snapped = e.altKey
        ? raw
        : magnetSnap(raw, drag.originalLengthTicks, applySnap(raw), edgeTargets(), reach)
      const effectiveDelta = snapped - drag.originalPositionTicks

      // Up or down: the clips go that many rows over, each staying on
      // its own channel, so it sounds the same on any row. Dragging to
      // another row used to move the clip to that row's track, through
      // that track's instrument and effects.
      const row = mouseY >= RULER_HEIGHT ? rowAt(mouseY) : -1
      const rowDelta = row >= 0
        ? Math.max(row - (drag.startRow ?? row), -(drag.originalRow ?? 0))
        : (drag.lastRowDelta ?? 0)
      drag.lastPositionTicks = snapped
      drag.lastDeltaTicks = effectiveDelta
      drag.lastRowDelta = rowDelta
      // Hot path, every mouse move: the store only, one update for the
      // whole group. The song gets the final places on release.
      const places = [{
        trackId: drag.trackId, clipId: drag.clipId, positionTicks: snapped,
        row: (drag.originalRow ?? 0) + rowDelta,
      }]
      for (const m of drag.groupMoveOriginals ?? []) {
        places.push({
          trackId: m.trackId, clipId: m.clipId, positionTicks: Math.max(0, m.origPos + effectiveDelta),
          row: Math.max(0, m.origRow + rowDelta),
        })
      }
      placeClipsLocal(places)
    } else if (drag.mode === 'resize-right') {
      const newLen = Math.max(minLen, drag.originalLengthTicks + dTicks)
      const start = drag.originalPositionTicks
      const end = e.altKey
        ? start + newLen
        : magnetEdge(start + newLen, start + applySnap(newLen), edgeTargets(), reach)
      drag.lastLengthTicks = Math.max(minLen, end - start)
      resizeClipLocal(drag.trackId, drag.clipId, drag.lastLengthTicks)
    } else if (drag.mode === 'resize-left') {
      // Keep the right edge anchored; shift position and shrink length.
      const rightEdge = drag.originalPositionTicks + drag.originalLengthTicks
      let newPos = Math.max(0, drag.originalPositionTicks + dTicks)
      newPos = e.altKey ? newPos : magnetEdge(newPos, applySnap(newPos), edgeTargets(), reach)
      if (newPos > rightEdge - minLen) newPos = rightEdge - minLen
      const newLen = rightEdge - newPos
      moveClipLocal(drag.trackId, drag.clipId, newPos)
      resizeClipLocal(drag.trackId, drag.clipId, newLen)
    }
  }, [hitTest, itemHit, getScrollOffset, pixelsPerTick, moveClipLocal, placeClipsLocal, resizeClipLocal, setClipFades, snapTicks, PIXELS_PER_SECOND, sampleRate, setPosition, onSetHint, trackHeight, verticalScroll, audioTracks, rowAt, paintClipAt, markers])

  const handleMouseUp = useCallback(() => {
    // Right-mouse-button pan release. If we panned (moved past 4 px)
    // the trailing onContextMenu must suppress its menu — `moved`
    // flag stays set until onContextMenu reads + clears it.
    if (rightPanRef.current) {
      // Leave the ref intact so onContextMenu can see `moved`. It
      // clears the ref itself after suppressing or showing the menu.
      return
    }
    const drag = dragRef.current
    // Pending empty-area click that never moved past the threshold: pure
    // no-op. No edit-cursor jump, no marquee, no selection change beyond
    // what mousedown already cleared (when ctrl wasn't held).
    if (drag && drag.mode === 'pending-empty') {
      dragRef.current = null
      return
    }
    // Paint released: every slot was already placed on the way, so there is
    // nothing to commit. The group closes once the placements it started have
    // landed, and only then does the drag become one undo entry.
    if (drag && drag.mode === 'paint') {
      const painted = drag.paintedSlots?.size ?? 0
      const chain = drag.paintChain ?? Promise.resolve()
      dragRef.current = null
      chain.finally(() => {
        useTrackStore.getState().endHistoryGroup(
          painted > 0 ? `Paint ${painted} clip${painted === 1 ? '' : 's'}` : null,
        )
      })
      return
    }
    // Loop-range drag commit: persist the final range to the engine and
    // enable looping if it wasn't already on. Zero-width drag (user did
    // ctrl+click without dragging) is a no-op so we don't accidentally
    // wipe the existing loop on a stray click.
    if (drag && drag.mode === 'loop-range') {
      const ts = useTransportStore.getState()
      if (ts.loopEnd > ts.loopStart) {
        setLoop(ts.loopStart, ts.loopEnd)
        if (!ts.looping) toggleLoop()
      }
      dragRef.current = null
      forceRender(n => n + 1)
      return
    }
    if (drag && drag.mode === 'slip') {
      // The group closes once every slide it sent has landed.
      const slid = (drag.slipSamples ?? 0) !== 0
      const chain = drag.paintChain ?? Promise.resolve()
      dragRef.current = null
      chain.finally(() => {
        useTrackStore.getState().endHistoryGroup(slid ? 'Slip clip' : null)
      })
      forceRender(n => n + 1)
      return
    }
    if (drag && drag.mode === 'rubber') {
      // Finalize rubber-band selection — any clip intersecting the box is selected.
      // mouseY values were recorded in viewport space; lift them into
      // logical (unscrolled) Y so we compare against each track's
      // y consistently regardless of the user's current scroll offset.
      const scrollOffset = getScrollOffset()
      const x1 = Math.min(drag.startMouseX, drag.currentMouseX)
      const x2 = Math.max(drag.startMouseX, drag.currentMouseX)
      const y1 = Math.min(drag.startMouseY, drag.currentMouseY)
      const y2 = Math.max(drag.startMouseY, drag.currentMouseY)
      // The rows the box covers, and on them what reaches into its bars.
      const rowFrom = Math.max(0, rowAt(Math.max(RULER_HEIGHT, y1)))
      const rowTo = rowAt(Math.max(RULER_HEIGHT, y2))
      const picked = pixelsPerTick > 0 && rowTo >= 0
        ? itemsInBox(rowIndex, rowFrom, rowTo, (x1 + scrollOffset) / pixelsPerTick, (x2 + scrollOffset) / pixelsPerTick)
          .filter(i => i.kind === 'clip')
          .map(i => i.clip.id)
        : []
      if (picked.length > 0) {
        useTrackStore.setState(s => {
          const next = new Set(s.selectedClipIds)
          for (const id of picked) next.add(id)
          return { selectedClipIds: next, selectedClipId: picked[0] }
        })
      }
    } else if (drag && drag.automation) {
      // An automation clip let go: its point, length, or place and row,
      // sent once, one undo step.
      const { trackId, clipId } = drag
      const store = useTrackStore.getState()
      const a = drag.automation
      let sent: Promise<unknown> | null = null
      if (a.pointIndex !== undefined && drag.fadesSent) {
        const [tick, value] = drag.fadesSent
        sent = store.moveAutomationClipPoint(trackId, clipId, a.pointIndex, tick, value)
      } else if (drag.mode === 'resize-right' && drag.lastLengthTicks !== undefined
        && drag.lastLengthTicks !== drag.originalLengthTicks) {
        sent = store.resizeAutomationClip(trackId, clipId, drag.lastLengthTicks)
      } else if (drag.mode === 'move' && a.pointIndex === undefined) {
        const startTick = drag.lastPositionTicks ?? drag.originalPositionTicks
        const row = Math.max(0, (drag.originalRow ?? 0) + (drag.lastRowDelta ?? 0))
        sent = store.placeAutomationClip(trackId, clipId, startTick, row)
      }
      ;(sent ?? store.refreshTrack(trackId)).catch(err => {
        console.error('automation clip edit failed', err)
        void store.refreshTrack(trackId)
      })
      if (sent) markEdited()
    } else if (drag && (drag.mode === 'fade-in' || drag.mode === 'fade-out')) {
      // Every fade sent on the way is in the drag's one undo step.
      closeDragStep(drag, 'Edit clip fades', drag.paintChain ?? Promise.resolve())
    } else if (drag && (drag.mode === 'move' || drag.mode === 'resize-right' || drag.mode === 'resize-left')) {
      // Flush the drag's optimistic local state to the backend in one
      // shot. We've been updating the store in-memory during mousemove
      // (moveClipLocal / resizeClipLocal) without IPC — now persist the
      // final position/length so save/load + audio engine see it.
      const liveTrack = useTrackStore.getState().tracks.find(t => t.id === drag.trackId)
      const liveClip = liveTrack?.clips?.find(c => c.id === drag.clipId)
      if (drag.mode === 'resize-left') {
        // A click on the edge that moved nothing is no edit.
        if (liveClip && liveClip.position_ticks !== drag.originalPositionTicks) {
          commitClipTrimStart(drag.trackId, drag.clipId, liveClip.position_ticks)
            .catch(err => console.error('commitClipTrimStart failed', err))
        }
      } else if (liveClip && drag.mode === 'move' && !drag.lastRowDelta
        && usePlaylistToolStore.getState().ripple
        && liveClip.position_ticks !== drag.originalPositionTicks) {
        // Ripple editing: the clips after the moved ones move with them,
        // as one undo step.
        void commitRippleMove(drag, liveClip.position_ticks - drag.originalPositionTicks)
          .catch(err => console.error('ripple move failed', err))
        dragRef.current = null
        forceRender(n => n + 1)
        return
      } else {
        // The dragged clip and its group, where the drag put them (their
        // places and rows), as one undo step. Nothing is sent for a click
        // that moved nothing.
        const delta = drag.lastDeltaTicks ?? 0
        const rowDelta = drag.mode === 'move' ? (drag.lastRowDelta ?? 0) : 0
        const edits: ClipDragEdit[] = [{
          trackId: drag.trackId,
          clipId: drag.clipId,
          fromPositionTicks: drag.sentPositionTicks ?? drag.originalPositionTicks,
          positionTicks: drag.lastPositionTicks ?? drag.originalPositionTicks,
          fromRow: drag.originalRow,
          row: drag.originalRow === undefined ? undefined : drag.originalRow + rowDelta,
          ...(drag.mode === 'resize-right'
            ? { fromLengthTicks: drag.originalLengthTicks, lengthTicks: drag.lastLengthTicks ?? drag.originalLengthTicks }
            : {}),
        }]
        for (const m of drag.groupMoveOriginals ?? []) {
          edits.push({
            trackId: m.trackId,
            clipId: m.clipId,
            fromPositionTicks: m.origPos,
            positionTicks: Math.max(0, m.origPos + delta),
            fromRow: m.origRow,
            row: Math.max(0, m.origRow + rowDelta),
          })
        }
        const onlyRows = rowDelta !== 0 && delta === 0
        const label = drag.mode === 'resize-right' ? 'Resize clip'
          : onlyRows
            ? (edits.length === 1 ? `Move clip to row ${(drag.originalRow ?? 0) + rowDelta + 1}` : `Move ${edits.length} clips to other rows`)
            : edits.length === 1 ? 'Move clip' : `Move ${edits.length} clips`
        const done = (drag.paintChain ?? Promise.resolve())
          .then(() => commitClipDrag(edits, label))
          .catch(err => console.error('commitClipDrag failed', err))
        closeDragStep(drag, label, done)
      }
    }
    dragRef.current = null
    forceRender(n => n + 1)
  }, [rowIndex, rowAt, pixelsPerTick, getScrollOffset, commitClipDrag, commitClipTrimStart])

  const handleDoubleClick = useCallback((e: React.MouseEvent) => {
    const rect = (e.target as HTMLElement).getBoundingClientRect()
    const mouseX = e.clientX - rect.left
    const mouseY = e.clientY - rect.top
    if (mouseY < RULER_HEIGHT) return
    const scrollOffset = getScrollOffset()
    const top = itemHit(mouseX, mouseY, scrollOffset)
    if (top && top.item.kind === 'auto' && top.point === undefined) {
      // A new point where the automation clip was double-clicked.
      const clip = top.item.clip as AutomationClipInfo
      const rowTop = RULER_HEIGHT + top.item.row * trackHeight - verticalScroll
      const h = trackHeight - 2
      const headerH = h >= 34 ? 15 : Math.min(12, Math.max(9, Math.round(h * 0.34)))
      const upper = rowTop + 1 + headerH + 2
      const lower = rowTop + trackHeight - 4
      const raw = (mouseX + scrollOffset) / pixelsPerTick - clip.startTick
      const tick = Math.max(0, Math.min(clip.lengthTicks, applySnap(Math.round(raw))))
      const value = lower > upper
        ? Math.max(0, Math.min(1, (lower - mouseY) / (lower - upper)))
        : automationValueAt(clip.points, tick)
      void useTrackStore.getState().addAutomationClipPoint(top.item.track.id, clip.id, tick, value)
        .then(() => markEdited())
      return
    }
    const hit = hitTest(mouseX, mouseY, scrollOffset)
    if (!hit) return
    if (hit.clip.kind === 'midi') {
      useTrackStore.getState().setActiveMidiClip(hit.trackId, hit.clip.id)
      window.dispatchEvent(new CustomEvent('daw:openPianoRoll'))
    } else if (hit.clip.kind === 'audio') {
      // A sample's settings, as FL opens a sample's channel settings.
      useSampleWindowStore.getState().open(hit.trackId, hit.clip.id)
    }
  }, [hitTest, itemHit, getScrollOffset, pixelsPerTick, trackHeight, verticalScroll, applySnap])

  const handleContextMenu = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    // Right-mouse-button pan release: if the user actually panned
    // (moved > 4 px) we swallow the context menu so the drag gesture
    // doesn't surprise them with a menu pop. A static right-click
    // (no drag) falls through to the normal context-menu code below.
    const rp = rightPanRef.current
    if (rp) {
      const moved = rp.moved
      rightPanRef.current = null
      if (moved) return
    }
    const rect = (e.target as HTMLElement).getBoundingClientRect()
    const mouseX = e.clientX - rect.left
    const mouseY = e.clientY - rect.top
    const scrollOffset = getScrollOffset()
    if (mouseY < RULER_HEIGHT) {
      const tick = Math.max(0, Math.round((mouseX + scrollOffset) / pixelsPerTick))
      const markerHit = markers.find(m => {
        const mx = m.tick * pixelsPerTick - scrollOffset
        return mouseX >= mx - 1 && mouseX <= mx + 11
      })
      // A section band under the pointer gets its own actions, so a
      // section is moved or renamed where it is drawn, not by typing its
      // name into a prompt.
      const sectionHit = markerHit ? null : sectionAt(sections, tick)
      setContextMenu(null)
      setMarkerCtx({
        x: e.clientX, y: e.clientY, markerId: markerHit ? markerHit.id : null,
        sectionId: sectionHit ? sectionHit.id : null, tick: applySnap(tick),
      })
      return
    }
    // An automation clip: right-click a point to remove it, the clip for
    // its own short menu.
    const top = itemHit(mouseX, mouseY, scrollOffset)
    if (top && top.item.kind === 'auto') {
      const trackId = top.item.track.id
      const clipId = top.item.clip.id
      setContextMenu(null)
      setMarkerCtx(null)
      if (top.point !== undefined) {
        void useTrackStore.getState().removeAutomationClipPoint(trackId, clipId, top.point).then(() => markEdited())
      } else {
        setAutoMenu({ x: e.clientX, y: e.clientY, trackId, clipId, name: automationClipName(top.item) })
      }
      return
    }
    const hit = hitTest(mouseX, mouseY, scrollOffset)
    if (hit) {
      if (!selectedClipIds.has(hit.clip.id)) {
        selectClip(hit.clip.id, hit.trackId)
      }
      setContextMenu({ x: e.clientX, y: e.clientY, trackId: hit.trackId, clipId: hit.clip.id })
      setMarkerCtx(null)
    } else {
      setContextMenu(null)
      setMarkerCtx(null)
    }
  }, [hitTest, itemHit, getScrollOffset, selectClip, selectedClipIds, markers, sections, pixelsPerTick, applySnap])

  // A sample dragged from the browser. The drag is a pointer drag (see
  // lib/pointerDrag): HTML5 drag events never reach the page on Windows.
  const dropBrowserFile = useCallback(async (path: string, clientX: number, clientY: number) => {
    const rect = containerRef.current?.getBoundingClientRect()
    if (!rect) return
    const mouseX = clientX - rect.left
    const mouseY = clientY - rect.top
    const scrollOffset = getScrollOffset()

    // The row under the pointer, at the bar under it. The sample gets an
    // audio clip channel of its own (the first time; it plays into the
    // next free insert) and its clip goes on that channel, on this row.
    // It used to land on the row's track, so the channel made for it held
    // nothing and its mute and volume did nothing to the sample.
    const row = Math.max(0, rowAt(Math.max(RULER_HEIGHT, mouseY)))
    const rawTicks = Math.max(0, Math.round((mouseX + scrollOffset) / pixelsPerTick))
    const positionTicks = applySnap(rawTicks)

    try {
      await useTrackStore.getState().placeSampleOnRow(path, row, positionTicks)
      const { useBrowserStore } = await import('../../stores/browserStore')
      useBrowserStore.getState().pushFileRecent(path)
    } catch (err) {
      console.error('browser drop import failed:', path, err)
      useNotificationStore.getState().push('error', tf('Could not add the sample: {error}', { error: String(err) }))
    }
  }, [pixelsPerTick, getScrollOffset, applySnap, rowAt])

  useDropTarget(containerRef, {
    accepts: (data) => data.startsWith('file:'),
    onOver: () => setDropHighlight(true),
    onLeave: () => setDropHighlight(false),
    onDrop: (d) => { void dropBrowserFile(d.data.slice('file:'.length), d.clientX, d.clientY) },
  })

  const zoomPending = useRef<{ factor: number; mouseX: number } | null>(null)
  const handleWheel = useCallback((e: React.WheelEvent) => {
    // Ctrl+Shift+Wheel: vertical zoom (change track height)
    if (e.ctrlKey && e.shiftKey) {
      e.preventDefault()
      const delta = e.deltaY < 0 ? 8 : -8
      setTrackHeight(trackHeight + delta)
      return
    }
    // Ctrl+Wheel: horizontal zoom, around the mouse. The moment under the
    // pointer stays under it, as in FL; zooming used to keep the left
    // edge still, so what you pointed at slid away.
    // Wheel steps arrive faster than frames, and each one redrew the whole
    // playlist: they are gathered and applied once a frame. The amount
    // follows the wheel, so a trackpad pinch zooms smoothly and a mouse
    // notch about 15 %.
    if (e.ctrlKey) {
      e.preventDefault()
      const rect = containerRef.current?.getBoundingClientRect()
      const mouseX = rect ? Math.max(0, e.clientX - rect.left) : 0
      const dy = e.deltaMode === 1 ? e.deltaY * 33 : e.deltaY
      // Bigger steps when zoomed in far (see zoom.ts), judged at the zoom
      // this frame will reach with what is already gathered.
      const at = useTransportStore.getState().horizontalZoom * (zoomPending.current?.factor ?? 1)
      const step = zoomStep(dy, at)
      const pending = zoomPending.current
      if (pending) {
        pending.factor *= step
        pending.mouseX = mouseX
        return
      }
      zoomPending.current = { factor: step, mouseX }
      requestAnimationFrame(() => {
        const z = zoomPending.current
        zoomPending.current = null
        if (!z) return
        const store = useTransportStore.getState()
        const secsAtMouse = (z.mouseX + getScrollOffset()) / (PIXELS_PER_SECOND_BASE * store.horizontalZoom)
        store.setHorizontalZoom(store.horizontalZoom * z.factor)
        // The store clamps the zoom, so read back what it became.
        const pps = PIXELS_PER_SECOND_BASE * useTransportStore.getState().horizontalZoom
        setScrollX(Math.max(0, secsAtMouse * pps - z.mouseX))
        if (followPlayhead) setFollowPlayhead(false)
      })
      return
    }
    // Plain wheel (no modifier) → scroll the track list vertically.
    // The container is overflow:hidden + the canvas is absolutely
    // positioned, so there's no native scroll to hand off to. We
    // maintain our own offset that every draw + hit-test path
    // subtracts from its logical Y. Shift+wheel also routes here so
    // trackpad users who only have horizontal wheel can still scroll.
    e.preventDefault()
    const container = containerRef.current
    if (!container) return
    const viewportH = container.clientHeight - RULER_HEIGHT
    // Shift+wheel, or a trackpad's horizontal axis, pans the timeline.
    // Panning by hand takes over from follow-playhead, the same way dragging a
    // scrollbar does in every other DAW; the toolbar toggle puts it back.
    const horizontalIntent = e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)
    if (horizontalIntent) {
      e.preventDefault()
      let dx = e.shiftKey && e.deltaX === 0 ? e.deltaY : e.deltaX
      if (e.deltaMode === 1) dx *= 16
      else if (e.deltaMode === 2) dx *= 400
      setScrollX((x) => Math.max(0, (followPlayhead ? getScrollOffset() : x) + dx))
      setFollowPlayhead(false)
      return
    }

    const maxScroll = Math.max(0, contentH - viewportH)
    if (maxScroll <= 0) return
    // Normalize deltaY across mouse / trackpad / line-mode wheels —
    // pure line-mode (deltaMode=1) is the worst offender, sending ~3
    // for a single notch which feels frozen if applied 1:1.
    let dy = e.deltaY
    if (e.deltaMode === 1) dy *= 16
    else if (e.deltaMode === 2) dy *= viewportH
    setVerticalScroll((v) => Math.max(0, Math.min(maxScroll, v + dy)))
  }, [trackHeight, setTrackHeight, horizontalZoom, setHorizontalZoom, audioTracks.length, followPlayhead, getScrollOffset, PIXELS_PER_SECOND, setFollowPlayhead])

  /** Run one section command from the ruler menu and refresh what it
   *  changed: the bands always, the clips when the music moved. `failure`
   *  is the whole message shown when it does not work. */
  const sectionAction = useCallback(async (
    failure: string, command: string, args: Record<string, unknown>, movesMusic: boolean,
  ) => {
    setMarkerCtx(null)
    try {
      await invoke(command, args)
      markEdited()
      // Moving music moves the markers after it in the song too.
      if (movesMusic) await Promise.all([useTrackStore.getState().fetchTracks(), reloadTimelineState()])
      window.dispatchEvent(new CustomEvent('daw:sectionsChanged'))
    } catch (e) {
      useNotificationStore.getState().push('warning', failure, { detail: String(e) })
    }
  }, [])

  // Close marker context menu on outside mousedown
  useEffect(() => {
    if (!markerCtx) return
    const close = (e: MouseEvent) => {
      const t = e.target as HTMLElement
      if (t.closest('[data-marker-ctx-menu]')) return
      setMarkerCtx(null)
    }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [markerCtx])

  /**
   * Keyboard use of the arrangement.
   *
   * The playlist is a canvas, so without this there is nothing to tab
   * to and nothing for a screen reader to read: the whole arrangement
   * was mouse-only. The canvas takes focus, the arrows walk the tracks
   * and the clips on them, and what the focus lands on is spoken.
   */
  const [spokenFocus, setSpokenFocus] = useState('')

  const speakClip = useCallback((trackIndex: number, clipIndex: number) => {
    const track = audioTracks[trackIndex]
    if (!track) return
    const clip = track.clips[clipIndex]
    if (!clip) {
      setSpokenFocus(tf('{track}, no clips', { track: track.name }))
      return
    }
    const bar = Math.floor(clip.position_ticks / (PPQ * 4)) + 1
    const bars = Math.max(1, Math.round(clip.length_ticks / (PPQ * 4)))
    const where = { track: track.name, n: clipIndex + 1, count: track.clips.length, bar, bars }
    setSpokenFocus(
      (bars === 1
        ? tf('{track}, clip {n} of {count}, bar {bar}, 1 bar', where)
        : tf('{track}, clip {n} of {count}, bar {bar}, {bars} bars', where))
      + `${clip.muted ? t(', muted') : ''}`,
    )
  }, [audioTracks])

  const handleCanvasKeyDown = useCallback((e: React.KeyboardEvent<HTMLCanvasElement>) => {
    if (e.altKey) return // marker navigation owns Alt
    const ts = useTrackStore.getState()
    const current = ts.selectedClipId
    let trackIndex = audioTracks.findIndex(t => t.clips.some(c => c.id === current))
    if (trackIndex < 0) trackIndex = 0
    const track = audioTracks[trackIndex]
    if (!track) return
    let clipIndex = track.clips.findIndex(c => c.id === current)

    const select = (ti: number, ci: number) => {
      const t = audioTracks[ti]
      if (!t) return
      const clip = t.clips[ci]
      ts.selectTrack(t.id)
      ts.selectClip(clip ? clip.id : null, t.id)
      speakClip(ti, ci)
    }

    switch (e.key) {
      case 'ArrowDown':
      case 'ArrowUp': {
        e.preventDefault()
        const next = Math.min(
          audioTracks.length - 1,
          Math.max(0, trackIndex + (e.key === 'ArrowDown' ? 1 : -1)),
        )
        select(next, 0)
        return
      }
      case 'ArrowRight':
      case 'ArrowLeft': {
        e.preventDefault()
        const step = e.key === 'ArrowRight' ? 1 : -1
        if ((e.ctrlKey || e.metaKey) && clipIndex >= 0) {
          // Nudge by the snap value rather than walking the clips; with
          // Shift, by one tick (about half a millisecond at 140 BPM). Every
          // selected clip moves when the focused one is part of the
          // selection, as one undo step.
          const clip = track.clips[clipIndex]
          const by = e.shiftKey ? 1 : snapTicks > 0 ? snapTicks : PPQ
          const ids = ts.selectedClipIds.has(clip.id) ? ts.selectedClipIds : new Set([clip.id])
          void ts.nudgeClips(ids, step * by)
          const to = Math.max(0, clip.position_ticks + step * by)
          setSpokenFocus(tf('moved to bar {bar}', { bar: Math.floor(to / (PPQ * 4)) + 1 }))
          return
        }
        if (clipIndex < 0) clipIndex = step > 0 ? -1 : track.clips.length
        const next = Math.min(track.clips.length - 1, Math.max(0, clipIndex + step))
        select(trackIndex, next)
        return
      }
      case 'Enter': {
        if (clipIndex < 0) return
        e.preventDefault()
        const clip = track.clips[clipIndex]
        if (clip.kind === 'midi') {
          ts.setActiveMidiClip(track.id, clip.id)
          window.dispatchEvent(new CustomEvent('daw:openPianoRoll'))
          setSpokenFocus(tf('opened {name} in the piano roll', { name: clip.name }))
        } else {
          setSpokenFocus(tf('{name} is audio, nothing to open', { name: clip.name }))
        }
        return
      }
      case 'Delete':
      case 'Backspace': {
        if (clipIndex < 0) return
        e.preventDefault()
        const name = track.clips[clipIndex].name
        void ts.deleteSelectedClips()
        setSpokenFocus(tf('removed {name}', { name }))
        return
      }
      default:
    }
  }, [audioTracks, snapTicks, speakClip])

  // Marker navigation hotkeys
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null
      if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) return
      const { positionSamples } = useTransportStore.getState()
      const playheadTicks = Math.round(tickAtPosition(positionSamples))
      if (e.altKey && e.code === 'ArrowRight') {
        e.preventDefault()
        const next = jumpToNext(playheadTicks)
        if (next) setPosition(positionAtTick(next.tick))
      } else if (e.altKey && e.code === 'ArrowLeft') {
        e.preventDefault()
        const prev = jumpToPrev(playheadTicks)
        if (prev) setPosition(positionAtTick(prev.tick))
      } else if (e.shiftKey && e.code === 'KeyM' && !e.ctrlKey && !e.metaKey) {
        e.preventDefault()
        addMarker(playheadTicks)
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [jumpToNext, jumpToPrev, addMarker, setPosition]) // eslint-disable-line react-hooks/exhaustive-deps

  // Drag-and-drop
  const [dropHighlight, setDropHighlight] = useState(false)
  // Where files dropped from the desktop go, from the drop's position in
  // the window (in device pixels): the row and bar under it, or with no
  // position, or one outside the playlist, the first empty row from the
  // start. Read through a ref by the listeners, which are set up once.
  const dropTargetRef = useRef<(at?: { x: number; y: number }) => { row: number; tick: number }>(() => ({ row: 0, tick: 0 }))
  dropTargetRef.current = (at) => {
    const rect = containerRef.current?.getBoundingClientRect()
    if (at && rect && pixelsPerTick > 0) {
      const dpr = window.devicePixelRatio || 1
      const x = at.x / dpr - rect.left
      const y = at.y / dpr - rect.top
      if (x >= 0 && y >= RULER_HEIGHT && x <= rect.width && y <= rect.height) {
        return {
          row: Math.max(0, rowAt(y)),
          tick: applySnap(Math.max(0, Math.round((x + getScrollOffset()) / pixelsPerTick))),
        }
      }
    }
    let row = 0
    while (rowIndex.byRow.has(row) || rowIndex.lanes.has(row)) row++
    return { row, tick: 0 }
  }

  useEffect(() => {
    const audioExts = ['wav', 'flac', 'mp3', 'ogg', 'aac', 'm4a', 'aiff', 'aif', 'aifc']
    const pushNotif = useNotificationStore.getState().push
    useLogStore.getState().append({
      level: 'event',
      test: 'drag-drop',
      message: 'listeners registered (tauri:// + daw: channels)',
    })

    // Native bridge: the Rust on_window_event re-emits drag/drop as
    // `daw:drag-drop` so we have a path that works on Windows + WebView2
    // even when the built-in tauri://drag-drop event is flaky there.
    // Both listeners stay registered; whichever fires first wins. The
    // import is idempotent enough that an accidental double-fire just
    // imports twice (unlikely — only one channel actually fires per drop).
    const log = useLogStore.getState().append
    const handleDropPayload = async (allPaths: string[], at?: { x: number; y: number }) => {
      setDropHighlight(false)
      console.log('[playlist] drag-drop received', allPaths.length, 'paths', allPaths)
      log({
        level: 'event',
        test: 'drag-drop',
        message: `drag-drop received ${allPaths.length} path(s)`,
        actual: allPaths.length === 0 ? '(empty)' : allPaths.join(', '),
      })
      const files = allPaths.filter(p => {
        const ext = p.split('.').pop()?.toLowerCase() || ''
        return audioExts.includes(ext)
      })
      if (allPaths.length > 0 && files.length === 0) {
        log({ level: 'fail', test: 'drag-drop', message: 'unsupported file format', actual: allPaths.join(', ') })
        pushNotif('warning', t('Dropped file is not a supported audio format'), {
          detail: tf('Supported: {formats}', { formats: audioExts.join(', ') }),
        })
        return
      }
      if (files.length === 0) {
        log({ level: 'event', test: 'drag-drop', message: 'no paths in payload (empty drop)' })
        return
      }

      // Where the files go: the row and bar under the pointer when the
      // drop says where it was, else the first empty row from the start.
      // One after another along the row, each on its own audio clip
      // channel (made the first time a file is dropped).
      const target = dropTargetRef.current(at)
      const row = target.row
      let offsetTicks = target.tick

      const { useBrowserStore } = await import('../../stores/browserStore')
      const pushRecent = useBrowserStore.getState().pushFileRecent
      let imported = 0
      for (const file of files) {
        try {
          const placed = await useTrackStore.getState().placeSampleOnRow(file, row, offsetTicks)
          offsetTicks += placed.clip.length_ticks
          pushRecent(file)
          imported++
          log({ level: 'pass', test: 'drag-drop', message: `imported ${file.split(/[\\/]/).pop() || file}`, actual: `+${placed.clip.length_ticks} ticks` })
        } catch (e) {
          console.error('Failed to import:', file, e)
          log({ level: 'fail', test: 'drag-drop', message: `import failed: ${file.split(/[\\/]/).pop() || file}`, actual: String(e) })
          pushNotif('error', tf('Failed to import {name}', { name: file.split(/[\\/]/).pop() ?? '' }), { detail: String(e) })
        }
      }
      if (imported > 0) {
        log({ level: 'info', test: 'drag-drop', message: `imported ${imported}/${files.length} file(s) on row ${row + 1}` })
        pushNotif('info', imported === files.length
          ? (imported === 1 ? t('Imported 1 file') : tf('Imported {n} files', { n: imported }))
          : tf('Imported {n} of {total} files', { n: imported, total: files.length }))
      }
    }

    // De-dupe: if both channels fire for the same drop within 200 ms,
    // ignore the second one. Windows users will likely receive the
    // native `daw:drag-drop` first; macOS / Linux users get the
    // standard `tauri://drag-drop` channel.
    let lastDropAt = 0
    const dispatchDrop = (paths: string[], at?: { x: number; y: number }) => {
      const now = Date.now()
      if (now - lastDropAt < 200) return
      lastDropAt = now
      void handleDropPayload(paths, at)
    }

    const unlistenDrop = listen<{ paths: string[]; position?: { x: number; y: number } }>('tauri://drag-drop', (event) => {
      log({ level: 'event', test: 'drag-drop', message: 'channel:tauri://drag-drop fired', actual: `${event.payload?.paths?.length ?? 0} path(s)` })
      dispatchDrop(event.payload?.paths ?? [], event.payload?.position)
    })
    const unlistenNativeDrop = listen<{ kind: string; paths?: string[]; x?: number; y?: number }>(
      'daw:drag-drop',
      (event) => {
        const payload = event.payload
        if (!payload) return
        if (payload.kind === 'enter' || payload.kind === 'over') setDropHighlight(true)
        else if (payload.kind === 'leave') setDropHighlight(false)
        else if (payload.kind === 'drop') {
          log({ level: 'event', test: 'drag-drop', message: 'channel:daw:drag-drop (native) fired', actual: `${payload.paths?.length ?? 0} path(s)` })
          dispatchDrop(payload.paths ?? [], payload.x !== undefined && payload.y !== undefined ? { x: payload.x, y: payload.y } : undefined)
        }
      },
    )
    const unlistenOver = listen('tauri://drag-over', () => setDropHighlight(true))
    const unlistenLeave = listen('tauri://drag-leave', () => setDropHighlight(false))

    // Browser-native fallback diagnostics. With dragDropEnabled:true on
    // tauri.conf, these should NOT fire (Tauri intercepts). If they do
    // fire, it means WebView2 is delivering the drop to the page but
    // Tauri's interceptor isn't catching it — that's a real signal.
    const winDragEnter = (e: DragEvent) => {
      e.preventDefault()
      log({ level: 'event', test: 'drag-drop', message: 'browser:dragenter (Tauri did NOT intercept)', actual: `types=${Array.from(e.dataTransfer?.types ?? []).join(',')}` })
    }
    const winDragOver = (e: DragEvent) => { e.preventDefault() }
    const winDrop = (e: DragEvent) => {
      e.preventDefault()
      const filesArr = Array.from(e.dataTransfer?.files ?? [])
      log({
        level: 'fail',
        test: 'drag-drop',
        message: 'browser:drop fired (Tauri bridge missed)',
        actual: filesArr.map(f => `${f.name} (${f.size}B)`).join(', ') || '(no files)',
      })
    }
    window.addEventListener('dragenter', winDragEnter)
    window.addEventListener('dragover', winDragOver)
    window.addEventListener('drop', winDrop)

    return () => {
      window.removeEventListener('dragenter', winDragEnter)
      window.removeEventListener('dragover', winDragOver)
      window.removeEventListener('drop', winDrop)
      unlistenDrop.then(f => f())
      unlistenNativeDrop.then(f => f())
      unlistenOver.then(f => f())
      unlistenLeave.then(f => f())
    }
  }, [])

  // The clip the context menu is for, and its colour. A colour picked there
  // goes to every selected clip when the clicked one is part of the
  // selection, as in FL Studio, and is saved with the song.
  const menuClip = contextMenu
    ? tracks.find(t => t.id === contextMenu.trackId)?.clips.find(c => c.id === contextMenu.clipId)
    : undefined
  const menuClipColor = menuClip?.color ?? null
  const colorMenuClips = (color: string | null) => {
    if (!contextMenu) return
    const ids = selectedClipIds.has(contextMenu.clipId) ? selectedClipIds : new Set([contextMenu.clipId])
    for (const track of tracks) {
      const onTrack = track.clips.filter(c => ids.has(c.id)).map(c => c.id)
      if (onTrack.length === 0) continue
      useTrackStore.getState().setClipColor(track.id, onTrack, color)
        .catch(e => useNotificationStore.getState().push('warning', t('Could not colour the clip'), { detail: String(e) }))
    }
  }

  return (
    <div
      ref={containerRef}
      style={{
        flex: 1,
        position: 'relative',
        overflow: 'hidden',
        // touch-action: none — block the browser's default pinch-zoom
        // and scroll on this canvas. The multi-touch hook (`useMultiTouchGestures`)
        // owns gesture detection and translates to our own zoom + pan
        // state, so the browser's defaults would only fight us.
        touchAction: 'none',
        // Background intentionally absent — the parent .fl-pl-grid renders
        // the mockup's purple gradient (#1a0e1f → #0e0716) and the canvas
        // is now transparent so it shows through.
        ...(dropHighlight ? { outline: `2px solid ${hw.accent}`, outlineOffset: -2 } : {}),
      }}
    >
      {/* What the keyboard is on, spoken for a screen reader. The
          arrangement is drawn on a canvas, which a reader cannot see
          into, so the canvas says out loud what it is showing. */}
      <div
        aria-live="polite"
        style={{
          position: 'absolute', width: 1, height: 1, overflow: 'hidden',
          clip: 'rect(0 0 0 0)', whiteSpace: 'nowrap',
        }}
      >{spokenFocus}</div>
      <canvas
        ref={canvasRef}
        data-testid="arrangement-canvas"
        data-track-height={trackHeight}
        tabIndex={0}
        role="application"
        aria-label={t('Arrangement. Arrow keys move between tracks and clips, Enter opens a clip, Delete removes it, Ctrl with left or right nudges it by the snap value.')}
        onKeyDown={handleCanvasKeyDown}
        style={{ position: 'absolute', top: 0, left: 0, outline: 'none' }}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
        onDoubleClick={handleDoubleClick}
        onContextMenu={handleContextMenu}
        onWheel={handleWheel}
      />
      {tempoLaneShown && (
        <TempoLane
          top={BAR_RULER_HEIGHT}
          pixelsPerTick={pixelsPerTick}
          getScrollOffset={getScrollOffset}
          snapTicks={snapTicks}
          segments={meterSegments}
          onWheel={handleWheel}
          onSetHint={onSetHint}
        />
      )}
      <div ref={playheadRef} aria-hidden style={PLAYHEAD_STYLE}>
        <div style={PLAYHEAD_GLOW} />
        <div style={PLAYHEAD_LINE} />
        <div style={PLAYHEAD_CARET} />
      </div>
      {dragRef.current && dragRef.current.mode === 'rubber' && (() => {
        const d = dragRef.current
        const x = Math.min(d.startMouseX, d.currentMouseX)
        const y = Math.min(d.startMouseY, d.currentMouseY)
        const w = Math.abs(d.currentMouseX - d.startMouseX)
        const h = Math.abs(d.currentMouseY - d.startMouseY)
        return (
          <div
            data-testid="rubber-band"
            style={{
              position: 'absolute', left: x, top: y, width: w, height: h,
              background: 'rgba(220,38,38,0.08)',
              border: '1px solid rgba(220,38,38,0.6)',
              pointerEvents: 'none',
            }}
          />
        )
      })()}
      {tip && (
        <div
          data-testid="clip-note"
          style={{
            position: 'fixed', left: Math.min(tip.x + 14, window.innerWidth - 370), top: tip.y + 90 > window.innerHeight ? tip.y - 78 : tip.y + 16, zIndex: 900, pointerEvents: 'none',
            background: 'rgba(18,18,24,0.97)', border: `1px solid ${hw.border}`, borderRadius: 6,
            padding: '7px 10px', maxWidth: 360, boxShadow: '0 8px 24px rgba(0,0,0,0.5)',
            font: '500 11.5px Inter, system-ui, sans-serif', color: '#e4e4e7', lineHeight: 1.45,
          }}
        >
          <div><b style={{ color: '#fff' }}>{tip.title}</b> <span style={{ color: '#a1a1aa' }}>· {tip.kind}</span></div>
          <div>{tip.through}</div>
          <div style={{ color: '#a1a1aa' }}>{tip.row}</div>
        </div>
      )}
      {autoMenu && (
        <div
          data-testid="automation-clip-menu"
          onMouseLeave={() => setAutoMenu(null)}
          style={{
            position: 'fixed', left: autoMenu.x, top: autoMenu.y,
            background: '#12121a', border: `1px solid ${hw.border}`,
            borderRadius: 6, padding: 4, minWidth: 200, zIndex: 1000,
            boxShadow: '0 8px 24px rgba(0,0,0,0.6)',
          }}
        >
          <div style={{ padding: '4px 10px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
            {autoMenu.name}
          </div>
          <MenuItem label={t('Delete automation clip')} onClick={async () => {
            const { trackId, clipId } = autoMenu
            setAutoMenu(null)
            await useTrackStore.getState().deleteAutomationClip(trackId, clipId)
            markEdited()
          }} />
        </div>
      )}
      {contextMenu && (() => {
        const menuTrack = tracks.find(t => t.id === contextMenu.trackId)
        const menuClip = menuTrack?.clips.find(c => c.id === contextMenu.clipId)
        return (
        <div
          data-testid="clip-context-menu"
          onMouseLeave={() => setContextMenu(null)}
          style={{
            position: 'fixed', left: contextMenu.x, top: contextMenu.y,
            background: '#12121a', border: `1px solid ${hw.border}`,
            borderRadius: 6, padding: 4, minWidth: 200, zIndex: 1000,
            boxShadow: '0 8px 24px rgba(0,0,0,0.6)',
          }}
        >
          {menuClip && (
            <div style={{ padding: '4px 10px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
              {menuClip.name}
            </div>
          )}
          <MenuItem label={t('Duplicate (Ctrl+D)')} onClick={async () => {
            await duplicateClip(contextMenu.trackId, contextMenu.clipId)
            setContextMenu(null)
          }} />
          <MenuItem label={t('Split at playhead (S)')} onClick={async () => {
            const { positionSamples } = useTransportStore.getState()
            const atTicks = Math.round(tickAtPosition(positionSamples))
            try { await splitClip(contextMenu.trackId, contextMenu.clipId, atTicks) } catch {}
            setContextMenu(null)
          }} />
          <MenuItem label={menuClip?.reversed ? t('Un-reverse') : t('Reverse audio')} onClick={async () => {
            await toggleClipReverse(contextMenu.trackId, contextMenu.clipId)
            setContextMenu(null)
          }} />
          {/* Each note moved by its own amount: a singer is sharp on one
              word and flat on the next. */}
          <MenuItem label={t('Paint something out…')} onClick={() => {
            const { trackId, clipId } = contextMenu
            setContextMenu(null)
            setSpectralTarget({ trackId, clipId })
          }} />
          <MenuItem label={t('Tune this take…')} onClick={async () => {
            const { trackId, clipId } = contextMenu
            setContextMenu(null)
            setTuneTarget({ trackId, clipId })
          }} />
          <MenuItem label={t('Separate stems (Pro)…')} onClick={() => {
            const { trackId, clipId } = contextMenu
            setContextMenu(null)
            setStemsTarget({ trackId, clipId, name: menuClip?.name ?? 'Clip' })
          }} />
          {/* A hummed line is the fastest way to get an idea down and the
              slowest to retype. */}
          <MenuItem label={t('Turn into notes (audio to MIDI)')} onClick={async () => {
            const { trackId, clipId } = contextMenu
            setContextMenu(null)
            try {
              const result = await invoke<{
                trackId: string; clipId: string; noteCount: number; lowest: number; highest: number
              }>('audio_clip_to_midi', { trackId, clipId })
              markEdited()
              await useTrackStore.getState().fetchTracks()
              useNotificationStore.getState().push('info',
                result.noteCount === 1 ? t('1 note written') : tf('{n} notes written', { n: result.noteCount }), {
                  detail: tf('{lowest} to {highest}, on a new track under this one. One voice at a time: a chord comes out as whichever note is loudest.',
                    { lowest: noteName(result.lowest), highest: noteName(result.highest) }),
                })
            } catch (e) {
              useNotificationStore.getState().push('warning', t('Could not follow that'), { detail: String(e) })
            }
          }} />
          {/* Warping: the engine has always been able to pin a clip's beats
              to the grid and nothing could ask it to. */}
          <MenuItem label={t('Warp beats to the grid')} onClick={async () => {
            const { trackId, clipId } = contextMenu
            setContextMenu(null)
            const grid = snapEnabled ? snapToTicks(snapValue, true) : 0
            try {
              const count = await invoke<number>('warp_clip_to_grid', {
                trackId, clipId, gridTicks: grid,
              })
              markEdited()
              await useTrackStore.getState().fetchTracks()
              useNotificationStore.getState().push('info', tf('Warped {n} beats to the grid', { n: count }))
            } catch (e) {
              useNotificationStore.getState().push('warning', t('Nothing to warp'), { detail: String(e) })
            }
          }} />
          <MenuItem label={t('Send to clip launcher')} onClick={async () => {
            const { trackId, clipId } = contextMenu
            setContextMenu(null)
            const push = useNotificationStore.getState().push
            try {
              const since = Date.now()
              const slot = await invoke<number>('send_clip_to_launcher', { trackId, clipId })
              markEdited()
              useHistoryStore.getState().push('Send clip to launcher', since)
              push('info', tf('In the clip launcher, scene {n}', { n: slot + 1 }))
            } catch (e) {
              push('warning', t('Could not send that clip to the launcher'), { detail: String(e) })
            }
          }} />
          <MenuItem label={t('Consolidate selected clips')} onClick={async () => {
            const { trackId } = contextMenu
            setContextMenu(null)
            // The span the selected clips cover on this track, glued into
            // one clip so the part moves as a piece.
            const ts = useTrackStore.getState()
            const track = ts.tracks.find(t => t.id === trackId)
            const chosen = (track?.clips ?? []).filter(c => selectedClipIds.has(c.id))
            const clips = chosen.length > 0
              ? chosen
              : (track?.clips ?? []).filter(c => c.id === contextMenu.clipId)
            if (clips.length === 0) return
            const from = Math.min(...clips.map(c => c.position_ticks))
            const to = Math.max(...clips.map(c => c.position_ticks + c.length_ticks))
            const push = useNotificationStore.getState().push
            push('info', t('Consolidating…'))
            try {
              const since = Date.now()
              await invoke('consolidate_track_range', { trackId, startTicks: from, endTicks: to })
              markEdited()
              // The engine took its undo snapshot; the history names the step.
              useHistoryStore.getState().push('Consolidate clips', since)
              await useTrackStore.getState().fetchTracks()
              push('info', tf('Consolidated {n} clips into one', { n: clips.length }))
            } catch (e) {
              push('warning', t('Could not consolidate'), { detail: String(e), sticky: true })
            }
          }} />
          <MenuItem label={t('Remove warping')} onClick={async () => {
            const { trackId, clipId } = contextMenu
            setContextMenu(null)
            try {
              await invoke('set_clip_warp_markers', { trackId, clipId, markers: [] })
              markEdited()
              await useTrackStore.getState().fetchTracks()
            } catch (e) {
              useNotificationStore.getState().push('warning', t('Could not remove the warping'), { detail: String(e) })
            }
          }} />
          <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
          {(() => {
            const gid = clipToGroup[contextMenu.clipId]
            const selectionIds = Array.from(selectedClipIds)
            const canGroup = !gid && selectionIds.length >= 2 && selectionIds.includes(contextMenu.clipId)
            return (
              <>
                {canGroup && (
                  <MenuItem label={tf('Group {n} clips', { n: selectionIds.length })} onClick={() => {
                    groupClipsAction(selectionIds)
                    setContextMenu(null)
                  }} />
                )}
                {!gid && !canGroup && (
                  <MenuItem label={t('Group (select 2+ clips first)')} onClick={() => setContextMenu(null)} />
                )}
                {gid && (
                  <MenuItem label={t('Ungroup this clip')} onClick={() => {
                    ungroupClipAction(contextMenu.clipId)
                    setContextMenu(null)
                  }} />
                )}
              </>
            )
          })()}
          <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
          <MenuItem label={t('Reset gain (0 dB)')} onClick={async () => {
            await setClipGain(contextMenu.trackId, contextMenu.clipId, 0)
            setContextMenu(null)
          }} />
          <MenuItem label={t('Reset fades')} onClick={async () => {
            await setClipFades(contextMenu.trackId, contextMenu.clipId, 0, 0)
            setContextMenu(null)
          }} />
          {(() => {
            const track = tracks.find(t => t.id === contextMenu.trackId)
            const clip = track?.clips.find(c => c.id === contextMenu.clipId)
            if (!clip || clip.kind !== 'audio') return null
            const curves: Array<{ value: FadeCurveKind; label: string }> = [
              { value: 'linear', label: 'Linear' },
              { value: 'equal_power', label: 'Equal power' },
              { value: 's_curve', label: 'S-curve' },
              { value: 'logarithmic', label: 'Logarithmic' },
            ]
            const fi = clip.fadeInCurve ?? 'linear'
            const fo = clip.fadeOutCurve ?? 'linear'
            return (
              <>
                <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
                <div style={{ padding: '4px 10px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
                  {t('Fade in curve')}
                </div>
                {curves.map(c => (
                  <MenuItem key={`fi_${c.value}`} label={`${c.value === fi ? '● ' : '   '}${t(c.label)}`}
                    onClick={async () => {
                      await setClipFadeCurves(contextMenu.trackId, contextMenu.clipId, c.value, fo)
                      setContextMenu(null)
                    }} />
                ))}
                <div style={{ padding: '4px 10px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
                  {t('Fade out curve')}
                </div>
                {curves.map(c => (
                  <MenuItem key={`fo_${c.value}`} label={`${c.value === fo ? '● ' : '   '}${t(c.label)}`}
                    onClick={async () => {
                      await setClipFadeCurves(contextMenu.trackId, contextMenu.clipId, fi, c.value)
                      setContextMenu(null)
                    }} />
                ))}
              </>
            )
          })()}
          <MenuItem label={t('Reset pitch (0 st)')} onClick={async () => {
            await setClipPitch(contextMenu.trackId, contextMenu.clipId, 0)
            setContextMenu(null)
          }} />
          <MenuItem label={t('Reset stretch (1.00×)')} onClick={async () => {
            await setClipStretch(contextMenu.trackId, contextMenu.clipId, 1)
            setContextMenu(null)
          }} />
          <MenuItem label={t('Rename…')} onClick={() => {
            const { trackId, clipId } = contextMenu
            setContextMenu(null)
            const name = window.prompt(t('Clip name:'), menuClip?.name ?? '')?.trim()
            if (name && name !== menuClip?.name) {
              useTrackStore.getState().renameClip(trackId, clipId, name)
                .catch(e => useNotificationStore.getState().push('warning', t('Could not rename the clip'), { detail: String(e) }))
            }
          }} />
          <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
          <div style={{ padding: '4px 10px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
            {t('Color')}
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(8, 1fr)', gap: 2, padding: '2px 6px 4px' }}>
            {CLIP_COLORS.map(c => {
              const active = menuClipColor === c.toLowerCase()
              return (
                <button key={c} title={c}
                  onClick={() => { colorMenuClips(c); setContextMenu(null) }}
                  style={{
                    width: 18, height: 18, borderRadius: 3, background: c,
                    border: active ? '2px solid #fff' : '1px solid rgba(255,255,255,0.12)',
                    cursor: 'pointer', padding: 0,
                  }}
                />
              )
            })}
          </div>
          <MenuItem
            label={t('Custom color…')}
            onClick={(e) => {
              const trigger = e?.currentTarget as HTMLElement | undefined
              const anchor = trigger?.getBoundingClientRect() ?? new DOMRect(0, 0, 0, 0)
              const recolor = colorMenuClips
              useColorPickerStore.getState().open({
                anchor,
                current: menuClipColor,
                onPick: (c) => recolor(c),
                onClear: () => recolor(null),
                title: t('Clip color'),
              })
              setContextMenu(null)
            }}
          />
          <MenuItem label={t('Clear color override')} onClick={() => {
            colorMenuClips(null)
            setContextMenu(null)
          }} />
          <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
          <MenuItem label={t('Delete')} danger onClick={async () => {
            await deleteClip(contextMenu.trackId, contextMenu.clipId)
            setContextMenu(null)
          }} />
        </div>
        )
      })()}
      {markerCtx && (() => {
        const hit = markerCtx.markerId ? markers.find(m => m.id === markerCtx.markerId) : null
        const section = !hit && markerCtx.sectionId ? sections.find(s => s.id === markerCtx.sectionId) ?? null : null
        const moves = section ? sectionMoves(sections, section.id) : null
        return (
          <div
            data-marker-ctx-menu
            onMouseDown={e => e.stopPropagation()}
            style={{
              position: 'fixed', left: markerCtx.x, top: markerCtx.y, zIndex: 10000,
              minWidth: 180, padding: 4,
              background: 'rgba(12,12,18,0.96)',
              border: `1px solid ${hw.borderLight}`,
              borderRadius: hw.radius.md,
              boxShadow: '0 8px 32px rgba(0,0,0,0.55)',
              backdropFilter: hw.blur.md,
            }}
          >
            <div style={{ padding: '4px 10px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
              {hit ? hit.label : section ? section.name : `Tick ${markerCtx.tick}`}
            </div>
            {section && moves && (
              <>
                <MenuItem label={t('Rename section…')} onClick={() => {
                  const next = window.prompt(tf('New name for "{name}"', { name: section.name }), section.name)?.trim()
                  if (!next || next === section.name) { setMarkerCtx(null); return }
                  void sectionAction(t('Could not rename that section'), 'rename_section', { id: section.id, name: next }, false)
                }} />
                {moves.earlier !== undefined && (
                  <MenuItem label={t('Move section earlier')} onClick={() => {
                    void sectionAction(t('Could not move that section'), 'move_section', { id: section.id, before: moves.earlier }, true)
                  }} />
                )}
                {moves.later !== undefined && (
                  <MenuItem label={t('Move section later')} onClick={() => {
                    void sectionAction(t('Could not move that section'), 'move_section', { id: section.id, before: moves.later }, true)
                  }} />
                )}
                <MenuItem label={t('Repeat section')} onClick={() => {
                  void sectionAction(t('Could not repeat that section'), 'duplicate_section', { id: section.id }, true)
                }} />
                <MenuItem label={t('Remove section (keeps the music)')} onClick={() => {
                  void sectionAction(t('Could not remove that section'), 'delete_section', { id: section.id }, false)
                }} />
                <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
              </>
            )}
            {!hit && (
              <>
                <MenuItem label={t('Add marker here')} onClick={() => {
                  addMarker(markerCtx.tick)
                  setMarkerCtx(null)
                }} />
                <MenuItem label={t('Add tempo change…')} onClick={() => {
                  const raw = window.prompt('Tempo (BPM):', String(bpmAt(tempoEntries, markerCtx.tick)))
                  if (raw == null) { setMarkerCtx(null); return }
                  const parsed = parseFloat(raw)
                  if (Number.isFinite(parsed) && parsed > 0) {
                    tempoActions().setTempoAt(markerCtx.tick, Math.max(20, Math.min(999, parsed)))
                      .catch(e => useNotificationStore.getState().push('warning', t('Could not change the tempo there'), { detail: String(e) }))
                  }
                  setMarkerCtx(null)
                }} />
                <MenuItem label={t('Add time signature change…')} onClick={() => {
                  const raw = window.prompt(t('Time signature (e.g. 4/4, 6/8, 7/8):'), '4/4')
                  if (raw == null) { setMarkerCtx(null); return }
                  const m = raw.trim().match(/^(\d+)\s*\/\s*(\d+)$/)
                  if (m) {
                    tempoActions().setSignatureAt(markerCtx.tick, parseInt(m[1], 10), parseInt(m[2], 10))
                      .catch(e => useNotificationStore.getState().push('warning', t('Could not change the time signature there'), { detail: String(e) }))
                  }
                  setMarkerCtx(null)
                }} />
                <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
                <MenuItem label={t('Set punch-in here')} onClick={() => {
                  setPunchIn(markerCtx.tick)
                  if (punchOutTicks != null && punchOutTicks <= markerCtx.tick) setPunchOut(null)
                  setMarkerCtx(null)
                }} />
                <MenuItem label={t('Set punch-out here')} onClick={() => {
                  setPunchOut(markerCtx.tick)
                  if (punchInTicks != null && punchInTicks >= markerCtx.tick) setPunchIn(null)
                  setMarkerCtx(null)
                }} />
                {looping && loopEnd > loopStart && (
                  <MenuItem label={t('Set punch range from loop')} onClick={() => {
                    setPunchRangeFromLoop()
                    setMarkerCtx(null)
                  }} />
                )}
                {(punchInTicks != null || punchOutTicks != null) && (
                  <MenuItem label={t('Clear punch range')} onClick={() => {
                    clearPunch()
                    setMarkerCtx(null)
                  }} />
                )}
              </>
            )}
            {hit && (
              <>
                {hit.kind === 'tempo' && (
                  <MenuItem label={t('Edit tempo…')} onClick={() => {
                    const raw = window.prompt('Tempo (BPM):', String(hit.bpm ?? bpm))
                    if (raw == null) { setMarkerCtx(null); return }
                    const parsed = parseFloat(raw)
                    const entry = flagEntry(hit.id)
                    if (entry && Number.isFinite(parsed) && parsed > 0) {
                      tempoActions().setEntryTempo(entry.index, Math.max(20, Math.min(999, parsed)))
                        .catch(e => useNotificationStore.getState().push('warning', t('Could not change the tempo'), { detail: String(e) }))
                    }
                    setMarkerCtx(null)
                  }} />
                )}
                {hit.kind === 'timesig' && (
                  <MenuItem label={t('Edit time signature…')} onClick={() => {
                    const current = `${hit.timeSigNum ?? 4}/${hit.timeSigDen ?? 4}`
                    const raw = window.prompt(t('Time signature (e.g. 4/4, 6/8):'), current)
                    if (raw == null) { setMarkerCtx(null); return }
                    const m = raw.trim().match(/^(\d+)\s*\/\s*(\d+)$/)
                    const entry = flagEntry(hit.id)
                    if (m && entry) {
                      tempoActions().setEntrySignature(entry.index, parseInt(m[1], 10), parseInt(m[2], 10))
                        .catch(e => useNotificationStore.getState().push('warning', t('Could not change the time signature'), { detail: String(e) }))
                    }
                    setMarkerCtx(null)
                  }} />
                )}
                {(!hit.kind || hit.kind === 'generic') && (
                  <MenuItem label={t('Rename')} onClick={() => {
                    setRenamingMarker({ id: hit.id, draft: hit.label })
                    setMarkerCtx(null)
                  }} />
                )}
                {(!hit.kind || hit.kind === 'generic') && (
                  <>
                    <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
                    <div style={{ padding: '4px 10px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
                      {t('Color')}
                    </div>
                    <div style={{ display: 'grid', gridTemplateColumns: 'repeat(6, 1fr)', gap: 2, padding: '2px 6px 4px' }}>
                      {['#3B82F6', '#10B981', '#F59E0B', '#A855F7', '#EC4899', '#06B6D4'].map(c => (
                        <button key={c} title={c}
                          onClick={() => { updateMarker(hit.id, { color: c }); setMarkerCtx(null) }}
                          style={{
                            width: 18, height: 18, borderRadius: 3, background: c,
                            border: hit.color === c ? '2px solid #fff' : '1px solid rgba(255,255,255,0.12)',
                            cursor: 'pointer', padding: 0,
                          }}
                        />
                      ))}
                    </div>
                  </>
                )}
                <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
                <MenuItem
                  label={hit.kind === 'tempo' ? t('Remove tempo change') : hit.kind === 'timesig' ? t('Remove time signature change') : t('Delete marker')}
                  danger
                  onClick={() => {
                    const entry = flagEntry(hit.id)
                    if (entry?.kind === 'tempo') void tempoActions().removeTempoChange(entry.index)
                    else if (entry?.kind === 'timesig') void tempoActions().removeSignatureChange(entry.index)
                    else removeMarker(hit.id)
                    setMarkerCtx(null)
                  }}
                />
              </>
            )}
          </div>
        )
      })()}
      {renamingMarker && (() => {
        const hit = markers.find(m => m.id === renamingMarker.id)
        if (!hit) return null
        const mx = hit.tick * pixelsPerTick - getScrollOffset() + 13
        const commit = () => {
          const trimmed = renamingMarker.draft.trim()
          if (trimmed && trimmed !== hit.label) updateMarker(hit.id, { label: trimmed })
          setRenamingMarker(null)
        }
        return (
          <input
            autoFocus
            value={renamingMarker.draft}
            onChange={e => setRenamingMarker({ id: hit.id, draft: e.target.value })}
            onBlur={commit}
            onKeyDown={e => {
              if (e.key === 'Enter') commit()
              else if (e.key === 'Escape') setRenamingMarker(null)
            }}
            style={{
              position: 'absolute', left: Math.max(4, mx), top: 2,
              width: 120, fontSize: 10, color: hw.textPrimary,
              background: 'rgba(0,0,0,0.85)', border: `1px solid ${hit.color}`,
              borderRadius: 2, padding: '1px 3px', outline: 'none', zIndex: 100,
            }}
          />
        )
      })()}
      {looping && loopEnd > loopStart && (
        <div
          data-testid="loop-region-overlay"
          style={{ position: 'absolute', top: 0, left: 0, width: 0, height: 0, pointerEvents: 'none' }}
        />
      )}
      {audioTracks.length === 0 && (
        <div style={{
          position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center',
          color: hw.textFaint, fontSize: 12, pointerEvents: 'none',
        }}>
          {t('Drop audio files here or add tracks from the toolbar')}
        </div>
      )}
      {spectralTarget && (
        <SpectralEditor
          trackId={spectralTarget.trackId}
          clipId={spectralTarget.clipId}
          onClose={() => setSpectralTarget(null)}
        />
      )}
      {tuneTarget && (
        <TuneDialog
          trackId={tuneTarget.trackId}
          clipId={tuneTarget.clipId}
          onClose={() => setTuneTarget(null)}
        />
      )}
      {stemsTarget && (
        <StemsDialog
          trackId={stemsTarget.trackId}
          clipId={stemsTarget.clipId}
          clipName={stemsTarget.name}
          onClose={() => setStemsTarget(null)}
        />
      )}
    </div>
  )
}

function MenuItem({
  label,
  onClick,
  danger,
}: {
  label: string
  onClick: (e?: React.MouseEvent<HTMLButtonElement>) => void
  danger?: boolean
}) {
  return (
    <button
      onClick={(e) => onClick(e)}
      onMouseEnter={e => { e.currentTarget.style.background = 'rgba(255,255,255,0.06)' }}
      onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
      style={{
        display: 'block', width: '100%', textAlign: 'left',
        padding: '5px 10px', fontSize: 11, color: danger ? '#EF4444' : '#e4e4e7',
        background: 'transparent', border: 'none', cursor: 'pointer', borderRadius: 4,
      }}
    >
      {label}
    </button>
  )
}


/** A MIDI number as a name, so a message can say "C3" rather than 48. */
function noteName(pitch: number): string {
  const names = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
  return `${names[pitch % 12]}${Math.floor(pitch / 12) - 1}`
}
