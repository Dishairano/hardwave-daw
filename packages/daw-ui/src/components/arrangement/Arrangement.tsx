import { useRef, useEffect, useState, useCallback } from 'react'
import { listen } from '@tauri-apps/api/event'
import { useTrackStore, ClipInfo, FadeCurveKind } from '../../stores/trackStore'
import { useTransportStore, snapToTicks } from '../../stores/transportStore'
import { useMarkerStore } from '../../stores/markerStore'
import { useClipGroupStore } from '../../stores/clipGroupStore'
import { useTrackFolderStore } from '../../stores/trackFolderStore'
import { useNotificationStore } from '../../stores/notificationStore'
import { usePickerStore } from '../../stores/pickerStore'
import { useLogStore } from '../../dev/logStore'
import { hw } from '../../theme'

const PPQ = 960
const PIXELS_PER_SECOND_BASE = 100
// (mockup-aligned)
const RESIZE_HANDLE_PX = 6
// Canvas-internal bar ruler — 22 px reserved at the top for bar numbers
// drawn in the same pixel-space as the grid lines underneath. The HTML
// ruler in HwApp.tsx is hidden via .fl-pl-ruler-hidden when the canvas
// owns the ruler so we never have two competing visual layers.
const RULER_HEIGHT = 22

type DragMode =
  | 'none'
  | 'move'
  | 'resize-right'
  | 'resize-left'
  | 'fade-in'
  | 'fade-out'
  | 'rubber'
  | 'scrub'
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
  groupMoveOriginals?: { clipId: string; trackId: string; origPos: number }[]
}

interface ContextMenuState {
  x: number
  y: number
  trackId: string
  clipId: string
}

// Cache keyed by `${sourceId}:${bucketTier}` so zooming in fetches a higher-resolution
// peak set instead of upscaling the existing one.
const waveformData = new Map<string, [number, number][]>()
const FADE_HANDLE_PX = 10
const HEADER_H = 14

function bucketTier(desired: number): number {
  // Quantise to powers of two so we don't hammer the backend on tiny zoom changes.
  const clamped = Math.max(100, Math.min(4000, Math.ceil(desired)))
  return 1 << Math.ceil(Math.log2(clamped))
}

// Hardwave clip palette — vibrant on near-black
const CLIP_COLORS = [
  '#DC2626', '#10B981', '#A855F7', '#F59E0B',
  '#3B82F6', '#EC4899', '#06B6D4', '#84CC16',
]

interface ArrangementProps {
  onSetHint?: (text: string) => void
}

export function Arrangement({ onSetHint }: ArrangementProps = {}) {
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
  const [verticalScroll, setVerticalScroll] = useState(0)

  const {
    tracks, selectedClipId, selectedClipIds, selectClip, toggleClipSelection, clearSelection,
    moveClip, moveClipLocal, resizeClipLocal, commitClipDrag,
    moveClipToTrack, resizeClip, getWaveformPeaks, duplicateClip, splitClip, deleteClip, setClipFades,
    setClipFadeCurves, toggleClipReverse, setClipGain, setClipPitch, setClipStretch,
  } = useTrackStore()
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null)
  const [markerCtx, setMarkerCtx] = useState<{ x: number; y: number; markerId: string | null; tick: number } | null>(null)
  const [renamingMarker, setRenamingMarker] = useState<{ id: string; draft: string } | null>(null)
  const {
    positionSamples, playing, bpm, sampleRate, setPosition, looping, loopStart, loopEnd,
    setLoop, toggleLoop,
    trackHeight, setTrackHeight, snapValue, snapEnabled, horizontalZoom, setHorizontalZoom,
    clipColorOverrides, editCursorTicks, setEditCursor, setClipColor,
    punchEnabled, punchInTicks, punchOutTicks, setPunchIn, setPunchOut, clearPunch, setPunchRangeFromLoop,
  } = useTransportStore()
  const { markers, addMarker, addTempoMarker, addTimeSigMarker, removeMarker, updateMarker, jumpToNext, jumpToPrev } = useMarkerStore()
  const clipToGroup = useClipGroupStore(s => s.clipToGroup)
  const groupColors = useClipGroupStore(s => s.groupColors)
  const groupClipsAction = useClipGroupStore(s => s.groupClips)
  const ungroupClipAction = useClipGroupStore(s => s.ungroupClip)

  const folders = useTrackFolderStore(s => s.folders)
  const hiddenTrackIds = new Set<string>()
  for (const f of folders) if (f.collapsed) for (const tid of f.trackIds) hiddenTrackIds.add(tid)
  const audioTracks = tracks.filter(t => t.kind !== 'Master' && !hiddenTrackIds.has(t.id))
  const PIXELS_PER_SECOND = PIXELS_PER_SECOND_BASE * horizontalZoom
  const beatsPerSecond = bpm / 60
  const pixelsPerBeat = PIXELS_PER_SECOND / beatsPerSecond
  const pixelsPerTick = pixelsPerBeat / PPQ
  const snapTicks = snapToTicks(snapValue, snapEnabled)
  const applySnap = (ticks: number): number => snapTicks > 0 ? Math.round(ticks / snapTicks) * snapTicks : ticks

  // Load waveforms. Multi-zoom tiering: cache by (sourceId, tier) so zooming in refetches
  // a higher-resolution peak set rather than stretching the existing buckets.
  useEffect(() => {
    for (const track of audioTracks) {
      for (const clip of track.clips) {
        if (!clip.source_id) continue
        const desired = Math.max(100, Math.ceil(clip.length_ticks * pixelsPerTick / 2))
        const tier = bucketTier(desired)
        const key = `${clip.source_id}:${tier}`
        if (!waveformData.has(key)) {
          getWaveformPeaks(clip.source_id, tier).then(peaks => {
            waveformData.set(key, peaks)
            forceRender(n => n + 1)
          })
        }
      }
    }
  }, [tracks, horizontalZoom])

  // Draw
  useEffect(() => {
    const canvas = canvasRef.current
    const container = containerRef.current
    if (!canvas || !container) return

    const dpr = window.devicePixelRatio || 1
    const rect = container.getBoundingClientRect()
    canvas.width = rect.width * dpr
    canvas.height = rect.height * dpr
    canvas.style.width = `${rect.width}px`
    canvas.style.height = `${rect.height}px`

    const ctx = canvas.getContext('2d')
    if (!ctx) return
    ctx.scale(dpr, dpr)

    const w = rect.width
    const h = rect.height

    const playheadSecs = sampleRate > 0 ? positionSamples / sampleRate : 0
    const scrollOffset = Math.max(0, playheadSecs * PIXELS_PER_SECOND - w * 0.25)

    // Background — solid mockup-canvas color (#15091a). Replaces the earlier
    // pure-black fill so the playlist matches the mockup look. The .fl-pl-grid
    // parent has a gradient too, but the mockup expects this color to cover it.
    ctx.fillStyle = '#15091a'
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
    if (RULER_HEIGHT > 0) {
      ctx.fillStyle = '#040406'
      ctx.fillRect(0, 0, w, RULER_HEIGHT)
      ctx.strokeStyle = 'rgba(255, 255, 255, 0.08)'
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(0, RULER_HEIGHT - 0.5)
      ctx.lineTo(w, RULER_HEIGHT - 0.5)
      ctx.stroke()
    }

    // Beat grid — HQ pixel-snapped, mockup palette.
    const pixelsPerBar = pixelsPerBeat * 4
    const startBeat = Math.floor(scrollOffset / pixelsPerBeat)
    for (let i = startBeat; i < startBeat + Math.ceil(w / pixelsPerBeat) + 2; i++) {
      const xRaw = i * pixelsPerBeat - scrollOffset
      const x = Math.floor(xRaw) + 0.5
      if (x < -1 || x > w + 1) continue

      const isBar = i % 4 === 0

      ctx.strokeStyle = isBar ? 'rgba(255,255,255,0.11)' : 'rgba(255,255,255,0.04)'
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(x, 0)
      ctx.lineTo(x, h)
      ctx.stroke()
    }

    // Bar number labels in the ruler band. One label every 4 bars so
    // they don't crowd at low zoom; per-bar labels at high zoom.
    if (RULER_HEIGHT > 0 && pixelsPerBar > 0) {
      const labelEvery = pixelsPerBar >= 80 ? 1 : pixelsPerBar >= 40 ? 2 : 4
      ctx.fillStyle = 'rgba(180, 180, 200, 0.7)'
      ctx.font = '600 10px "JetBrains Mono", ui-monospace, Consolas, monospace'
      ctx.textBaseline = 'middle'
      const startBar = Math.max(0, Math.floor(scrollOffset / pixelsPerBar))
      const endBar = startBar + Math.ceil(w / pixelsPerBar) + 1
      for (let bar = startBar; bar <= endBar; bar++) {
        if (bar % labelEvery !== 0) continue
        const xRaw = bar * pixelsPerBar - scrollOffset
        const x = Math.floor(xRaw) + 4
        if (x < -20 || x > w) continue
        ctx.fillText(String(bar + 1), x, RULER_HEIGHT / 2 + 1)
      }
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

    // Clips. The y for each track shifts up by `verticalScroll` so the
    // visible band slides under the ruler. Tracks fully scrolled off
    // the bottom or top of the viewport are skipped at the for-loop
    // boundaries below for the trivial paint-cost win.
    for (let i = 0; i < audioTracks.length; i++) {
      const y = RULER_HEIGHT + i * trackHeight - verticalScroll
      // Cull tracks that fall entirely outside the visible viewport.
      if (y + trackHeight < RULER_HEIGHT) continue
      if (y > h) break
      const track = audioTracks[i]
      const defaultColor = CLIP_COLORS[i % CLIP_COLORS.length]

      for (const clip of track.clips) {
        const color = clipColorOverrides[clip.id] || defaultColor
        drawClip(ctx, clip, color, y, scrollOffset, w, pixelsPerTick)
      }
    }

    // Crossfade overlay — shade any overlap between adjacent clips on the same track.
    for (let i = 0; i < audioTracks.length; i++) {
      const y = RULER_HEIGHT + i * trackHeight - verticalScroll
      if (y + trackHeight < RULER_HEIGHT) continue
      if (y > h) break
      const clips = [...audioTracks[i].clips].sort((a, b) => a.position_ticks - b.position_ticks)
      for (let j = 1; j < clips.length; j++) {
        const prev = clips[j - 1]
        const cur = clips[j]
        const prevEnd = prev.position_ticks + prev.length_ticks
        if (cur.position_ticks < prevEnd) {
          const overlapTicks = prevEnd - cur.position_ticks
          const xStart = cur.position_ticks * pixelsPerTick - scrollOffset
          const width = overlapTicks * pixelsPerTick
          if (xStart + width >= 0 && xStart <= w) {
            ctx.save()
            ctx.fillStyle = 'rgba(20, 184, 166, 0.12)'
            ctx.fillRect(xStart, y + 2, width, trackHeight - 4)
            ctx.strokeStyle = 'rgba(20, 184, 166, 0.45)'
            ctx.lineWidth = 1
            // X mark
            ctx.beginPath()
            ctx.moveTo(xStart, y + 2)
            ctx.lineTo(xStart + width, y + trackHeight - 2)
            ctx.moveTo(xStart + width, y + 2)
            ctx.lineTo(xStart, y + trackHeight - 2)
            ctx.stroke()
            ctx.restore()
          }
        }
      }
    }

    // Loop region overlay
    if (looping && loopEnd > loopStart && sampleRate > 0) {
      const loopStartSecs = loopStart / sampleRate
      const loopEndSecs = loopEnd / sampleRate
      const loopX1 = loopStartSecs * PIXELS_PER_SECOND - scrollOffset
      const loopX2 = loopEndSecs * PIXELS_PER_SECOND - scrollOffset

      // Shaded region across all tracks
      ctx.fillStyle = 'rgba(220, 38, 38, 0.06)'
      ctx.fillRect(loopX1, RULER_HEIGHT, loopX2 - loopX1, h - RULER_HEIGHT)

      // Ruler highlight
      ctx.fillStyle = 'rgba(220, 38, 38, 0.15)'
      ctx.fillRect(loopX1, 0, loopX2 - loopX1, RULER_HEIGHT)

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
      ctx.lineTo(loopX1 + 6, RULER_HEIGHT / 2)
      ctx.lineTo(loopX1, RULER_HEIGHT)
      ctx.closePath()
      ctx.fill()
      // End marker — left-pointing triangle
      ctx.beginPath()
      ctx.moveTo(loopX2, 0)
      ctx.lineTo(loopX2 - 6, RULER_HEIGHT / 2)
      ctx.lineTo(loopX2, RULER_HEIGHT)
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
      ctx.fillRect(pX1, 0, pX2 - pX1, RULER_HEIGHT)

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
      ctx.moveTo(pX1 + 6, 2); ctx.lineTo(pX1, 2); ctx.lineTo(pX1, RULER_HEIGHT - 2); ctx.lineTo(pX1 + 6, RULER_HEIGHT - 2)
      ctx.moveTo(pX2 - 6, 2); ctx.lineTo(pX2, 2); ctx.lineTo(pX2, RULER_HEIGHT - 2); ctx.lineTo(pX2 - 6, RULER_HEIGHT - 2)
      ctx.stroke()
    }

    // Playhead — bright red, sharper 1px stroke with glow underlay (mockup style)
    const playheadX = Math.floor(playheadSecs * PIXELS_PER_SECOND - scrollOffset) + 0.5
    if (playing || positionSamples > 0) {
      // Glow underneath the sharp line
      ctx.strokeStyle = 'rgba(239,68,68,0.20)'
      ctx.lineWidth = 5
      ctx.beginPath()
      ctx.moveTo(playheadX, 0)
      ctx.lineTo(playheadX, h)
      ctx.stroke()

      ctx.strokeStyle = '#EF4444'
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(playheadX, 0)
      ctx.lineTo(playheadX, h)
      ctx.stroke()

      ctx.fillStyle = '#EF4444'
      ctx.beginPath()
      ctx.moveTo(playheadX - 5, 0)
      ctx.lineTo(playheadX + 5, 0)
      ctx.lineTo(playheadX, 8)
      ctx.closePath()
      ctx.fill()
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
        const pillH = Math.min(RULER_HEIGHT - 3, 14)
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
        ctx.lineTo(mx + 10, RULER_HEIGHT - 10)
        ctx.lineTo(mx + 4, RULER_HEIGHT - 6)
        ctx.lineTo(mx, RULER_HEIGHT - 10)
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
        ctx.moveTo(ecX - 4, RULER_HEIGHT - 6)
        ctx.lineTo(ecX + 4, RULER_HEIGHT - 6)
        ctx.lineTo(ecX, RULER_HEIGHT)
        ctx.closePath()
        ctx.fill()
      }
    }

  }, [tracks, positionSamples, playing, bpm, sampleRate, selectedClipId, selectedClipIds, looping, loopStart, loopEnd, trackHeight, horizontalZoom, snapValue, snapEnabled, clipColorOverrides, editCursorTicks, markers, renamingMarker, clipToGroup, groupColors, punchEnabled, punchInTicks, punchOutTicks, verticalScroll])

  function drawClip(
    ctx: CanvasRenderingContext2D,
    clip: ClipInfo,
    baseColor: string,
    trackY: number,
    scrollOffset: number,
    viewWidth: number,
    pxPerTick: number,
  ) {
    const clipX = clip.position_ticks * pxPerTick - scrollOffset
    const clipW = clip.length_ticks * pxPerTick
    if (clipX + clipW < 0 || clipX > viewWidth) return

    const pad = 1
    const x = clipX
    const y = trackY + pad
    const w = clipW
    const h = trackHeight - pad * 2
    const isSelected = clip.id === selectedClipId || selectedClipIds.has(clip.id)
    const color = clip.muted ? '#1a1a24' : baseColor
    // Mockup-style: sharp 1px corners, thin proportional header, full-saturation header stripe.
    const radius = 1
    const headerH = Math.min(9, Math.max(7, Math.round(h * 0.4)))

    // Clip body — fuller saturation than before
    ctx.fillStyle = color
    ctx.globalAlpha = clip.muted ? 0.25 : 0.35
    ctx.beginPath()
    ctx.roundRect(x, y, w, h, radius)
    ctx.fill()
    ctx.globalAlpha = 1.0

    // Clip header bar — full-color stripe
    ctx.fillStyle = clip.muted ? '#161620' : color
    ctx.globalAlpha = clip.muted ? 0.5 : 1.0
    ctx.beginPath()
    ctx.roundRect(x, y, w, headerH, [radius, radius, 0, 0])
    ctx.fill()
    ctx.globalAlpha = 1

    // Clip name — only render if header is tall enough and clip is wide enough
    if (headerH >= 8 && w >= 16) {
      ctx.fillStyle = clip.muted ? '#52525b' : 'rgba(0,0,0,0.85)'
      ctx.font = "700 7px 'JetBrains Mono', ui-monospace, Menlo, monospace"
      ctx.save()
      ctx.beginPath()
      ctx.rect(x + 2, y, w - 4, headerH)
      ctx.clip()
      ctx.fillText(clip.name, x + 3, y + headerH - 2)
      ctx.restore()
    }

    // Waveform — pick the best available tier: prefer the current one, else fallback
    // to any other tier we've already fetched so we render something during refetch.
    const desired = Math.max(100, Math.ceil(clip.length_ticks * pxPerTick / 2))
    const tier = bucketTier(desired)
    let peaks = waveformData.get(`${clip.source_id}:${tier}`)
    if (!peaks) {
      for (const [k, v] of waveformData) {
        if (k.startsWith(`${clip.source_id}:`)) { peaks = v; break }
      }
    }
    if (peaks && peaks.length > 0 && !clip.muted) {
      ctx.save()
      ctx.beginPath()
      ctx.rect(x + 1, y + headerH, w - 2, h - headerH - 1)
      ctx.clip()

      const waveArea = h - headerH - 2
      const midY = y + headerH + waveArea * 0.5
      const ampScale = waveArea * 0.45
      ctx.fillStyle = lightenColor(color, 0.4)
      ctx.globalAlpha = 0.6

      const pxPerBucket = w / peaks.length
      for (let j = 0; j < peaks.length; j++) {
        const bx = x + j * pxPerBucket
        if (bx + pxPerBucket < 0 || bx > viewWidth) continue
        const [minVal, maxVal] = peaks[j]
        const top = midY - maxVal * ampScale
        const bottom = midY - minVal * ampScale
        const barH = Math.max(0.5, bottom - top)
        ctx.fillRect(bx, top, Math.max(0.5, pxPerBucket - 0.3), barH)
      }

      ctx.globalAlpha = 1.0
      ctx.restore()
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

    // Border
    if (isSelected) {
      ctx.strokeStyle = '#EF4444'
      ctx.lineWidth = 2
      ctx.shadowColor = 'rgba(220,38,38,0.4)'
      ctx.shadowBlur = 8
    } else {
      ctx.strokeStyle = clip.muted ? '#2a2a38' : `${color}88`
      ctx.lineWidth = 1
      ctx.shadowBlur = 0
    }
    ctx.beginPath()
    ctx.roundRect(x, y, w, h, 6)
    ctx.stroke()
    ctx.shadowBlur = 0

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

  // Hit test. The incoming mouseY is viewport-relative; we convert it
  // to logical (unscrolled) Y once and compare against each track's
  // y from there, so the user's wheel-scroll position stays
  // transparent to every caller.
  const hitTest = useCallback((mouseX: number, mouseY: number, scrollOffset: number): {
    clip: ClipInfo, trackId: string, edge: 'body' | 'left' | 'right' | 'fade-in' | 'fade-out'
  } | null => {
    const logicalY = mouseY + verticalScroll
    for (let i = 0; i < audioTracks.length; i++) {
      const y = RULER_HEIGHT + i * trackHeight
      if (logicalY < y + 2 || logicalY > y + trackHeight - 2) continue
      const track = audioTracks[i]
      for (const clip of track.clips) {
        const clipX = clip.position_ticks * pixelsPerTick - scrollOffset
        const clipW = clip.length_ticks * pixelsPerTick
        if (mouseX < clipX || mouseX > clipX + clipW) continue
        const localY = logicalY - (y + 2)
        const withinFadeBand = localY >= HEADER_H && localY <= HEADER_H + FADE_HANDLE_PX && clipW > FADE_HANDLE_PX * 3
        // Fade handles take priority over resize edges within the narrow top band.
        if (withinFadeBand) {
          if (mouseX <= clipX + RESIZE_HANDLE_PX + FADE_HANDLE_PX) {
            return { clip, trackId: track.id, edge: 'fade-in' }
          }
          if (mouseX >= clipX + clipW - RESIZE_HANDLE_PX - FADE_HANDLE_PX) {
            return { clip, trackId: track.id, edge: 'fade-out' }
          }
        }
        let edge: 'body' | 'left' | 'right' = 'body'
        if (mouseX > clipX + clipW - RESIZE_HANDLE_PX) edge = 'right'
        else if (mouseX < clipX + RESIZE_HANDLE_PX) edge = 'left'
        return { clip, trackId: track.id, edge }
      }
    }
    return null
  }, [audioTracks, pixelsPerTick, trackHeight, verticalScroll])

  const getScrollOffset = useCallback(() => {
    const playheadSecs = sampleRate > 0 ? positionSamples / sampleRate : 0
    const container = containerRef.current
    const w = container ? container.getBoundingClientRect().width : 800
    return Math.max(0, playheadSecs * PIXELS_PER_SECOND - w * 0.25)
  }, [positionSamples, sampleRate])

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    if (e.button === 2) return // right-click handled by onContextMenu
    setContextMenu(null)
    const rect = (e.target as HTMLElement).getBoundingClientRect()
    const mouseX = e.clientX - rect.left
    const mouseY = e.clientY - rect.top
    const scrollOffset = getScrollOffset()

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
        const secs = markerHit.tick / PPQ / (bpm / 60)
        setPosition(Math.round(secs * (sampleRate || 48000)))
        return
      }
      const seconds = Math.max(0, (mouseX + scrollOffset) / PIXELS_PER_SECOND)
      const samples = Math.round(seconds * (sampleRate || 48000))
      setPosition(samples)
      dragRef.current = {
        mode: 'scrub', clipId: '', trackId: '',
        startMouseX: mouseX, startMouseY: mouseY, currentMouseX: mouseX, currentMouseY: mouseY,
        originalPositionTicks: 0, originalLengthTicks: 0,
        originalFadeInTicks: 0, originalFadeOutTicks: 0,
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
          useTrackStore.setState(s => {
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
      let groupMoveOriginals: { clipId: string; trackId: string; origPos: number }[] | undefined
      if (mode === 'move' && gid && groupMemberIds.length > 1) {
        groupMoveOriginals = []
        for (const t of tracks) {
          for (const c of t.clips) {
            if (c.id === hit.clip.id) continue
            if (groupMemberIds.includes(c.id)) {
              groupMoveOriginals.push({ clipId: c.id, trackId: t.id, origPos: c.position_ticks })
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
      }
    } else {
      // FL Studio place-mode: if the picker has an audio clip armed,
      // left-click in a track row places a copy at the clicked tick.
      // The rubber-band fallback only runs when no picker selection
      // is active (so existing select-multiple workflow is untouched).
      const pickerSel = usePickerStore.getState().selection
      if (pickerSel?.kind === 'audioClip') {
        const trackIdx = Math.floor((mouseY - RULER_HEIGHT) / trackHeight)
        const targetTrack = trackIdx >= 0 ? audioTracks[trackIdx] : undefined
        if (targetTrack) {
          const tickAt = applySnap(
            Math.max(0, Math.round((mouseX + scrollOffset) / pixelsPerTick)),
          )
          useTrackStore
            .getState()
            .placeClipCopy(pickerSel.trackId, pickerSel.clipId, targetTrack.id, tickAt)
            .catch(err => console.error('placeClipCopy failed', err))
          return
        }
      }
      if (!(e.ctrlKey || e.metaKey)) clearSelection()
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
  }, [hitTest, selectClip, toggleClipSelection, clearSelection, selectedClipIds, getScrollOffset, PIXELS_PER_SECOND, sampleRate, setPosition, pixelsPerTick, setEditCursor, snapTicks, clipToGroup, tracks, markers, bpm])

  const handleMouseMove = useCallback((e: React.MouseEvent) => {
    const drag = dragRef.current
    if (!drag || drag.mode === 'none') {
      const rect = (e.target as HTMLElement).getBoundingClientRect()
      const mouseX = e.clientX - rect.left
      const mouseY = e.clientY - rect.top
      const hit = hitTest(mouseX, mouseY, getScrollOffset())
      const canvas = canvasRef.current
      if (canvas) {
        if (mouseY < RULER_HEIGHT) {
          canvas.style.cursor = 'pointer'
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
          onSetHint('Timeline ruler — click to place playhead')
        } else if (hit) {
          const tip = hit.edge === 'right' || hit.edge === 'left'
            ? 'drag to resize · alt to bypass snap'
            : hit.edge === 'fade-in'
              ? 'drag to set fade-in length'
              : hit.edge === 'fade-out'
                ? 'drag to set fade-out length'
                : 'drag to move · alt to bypass snap'
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
      // Convert tick → samples using the same path the canvas uses to
      // draw the loop region, so the visual preview matches whatever the
      // engine will eventually loop.
      const aSec = (a / PPQ) * (60 / Math.max(1, bpm))
      const bSec = (b / PPQ) * (60 / Math.max(1, bpm))
      const sr = sampleRate || 48000
      // Live preview only — don't IPC on every mousemove. The commit
      // happens once on mouseup via transport.setLoop.
      useTransportStore.setState({
        loopStart: Math.round(aSec * sr),
        loopEnd: Math.round(bSec * sr),
      })
      forceRender(n => n + 1)
      return
    }
    if (drag.mode === 'scrub') {
      const scrollOffset = getScrollOffset()
      const seconds = Math.max(0, (mouseX + scrollOffset) / PIXELS_PER_SECOND)
      setPosition(Math.round(seconds * (sampleRate || 48000)))
      return
    }

    if (drag.mode === 'rubber') {
      forceRender(n => n + 1)
      return
    }

    if (drag.mode === 'fade-in') {
      const newFade = Math.max(0, Math.min(drag.originalLengthTicks, drag.originalFadeInTicks + dTicks))
      setClipFades(drag.trackId, drag.clipId, newFade, drag.originalFadeOutTicks)
      return
    }
    if (drag.mode === 'fade-out') {
      // Dragging left shortens fade-out from the right edge, so invert dx.
      const newFade = Math.max(0, Math.min(drag.originalLengthTicks, drag.originalFadeOutTicks - dTicks))
      setClipFades(drag.trackId, drag.clipId, drag.originalFadeInTicks, newFade)
      return
    }
    if (drag.mode === 'move') {
      const snapped = applySnap(Math.max(0, drag.originalPositionTicks + dTicks))
      const effectiveDelta = snapped - drag.originalPositionTicks

      // Cross-track move — hit-test the cursor's vertical position
      // against the visible audio-track stack. If the cursor lands
      // inside a different track row than where the drag started, ship
      // the clip to that track instead of just shifting position.
      // Refuses on type mismatch (audio→midi or vice-versa); the
      // backend command also enforces this with a hard error.
      // Skip the ruler band so a near-top mouseY does not register as
      // track index -1 / 0 incorrectly.
      const trackIdx = mouseY >= RULER_HEIGHT
        ? Math.floor((mouseY - RULER_HEIGHT) / trackHeight)
        : -1
      const targetTrack = trackIdx >= 0 ? audioTracks[trackIdx] : undefined
      const sourceTrack = audioTracks.find(t => t.id === drag.trackId)
      const canMoveAcross =
        targetTrack &&
        sourceTrack &&
        targetTrack.id !== drag.trackId &&
        targetTrack.kind === sourceTrack.kind

      if (canMoveAcross) {
        // Cross-track move still goes through the IPC path because it
        // needs to mutate Rust-side track membership, not just a clip
        // position. Rare event (one-off per drag), so the cost is OK.
        moveClipToTrack(drag.trackId, targetTrack.id, drag.clipId, snapped)
        // Update the drag ref so subsequent mousemoves treat the new
        // track as the source (otherwise the clip would teleport back
        // every frame).
        drag.trackId = targetTrack.id
      } else {
        // Hot path — fires every mousemove. Local-only update keeps the
        // 60fps drag fluid even with 500 pre-allocated tracks. The
        // backend gets the final position via commitClipDrag in
        // handleMouseUp.
        moveClipLocal(drag.trackId, drag.clipId, snapped)
      }
      if (drag.groupMoveOriginals) {
        for (const m of drag.groupMoveOriginals) {
          const memberPos = Math.max(0, m.origPos + effectiveDelta)
          moveClipLocal(m.trackId, m.clipId, memberPos)
        }
      }
    } else if (drag.mode === 'resize-right') {
      const newLen = Math.max(minLen, drag.originalLengthTicks + dTicks)
      resizeClipLocal(drag.trackId, drag.clipId, applySnap(newLen))
    } else if (drag.mode === 'resize-left') {
      // Keep the right edge anchored; shift position and shrink length.
      const rightEdge = drag.originalPositionTicks + drag.originalLengthTicks
      let newPos = Math.max(0, drag.originalPositionTicks + dTicks)
      newPos = applySnap(newPos)
      if (newPos > rightEdge - minLen) newPos = rightEdge - minLen
      const newLen = rightEdge - newPos
      moveClipLocal(drag.trackId, drag.clipId, newPos)
      resizeClipLocal(drag.trackId, drag.clipId, newLen)
    }
  }, [hitTest, getScrollOffset, pixelsPerTick, moveClipLocal, moveClipToTrack, resizeClipLocal, setClipFades, snapTicks, PIXELS_PER_SECOND, sampleRate, setPosition, onSetHint, trackHeight, audioTracks])

  const handleMouseUp = useCallback(() => {
    const drag = dragRef.current
    // Pending empty-area click that never moved past the threshold: pure
    // no-op. No edit-cursor jump, no marquee, no selection change beyond
    // what mousedown already cleared (when ctrl wasn't held).
    if (drag && drag.mode === 'pending-empty') {
      dragRef.current = null
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
    if (drag && drag.mode === 'rubber') {
      // Finalize rubber-band selection — any clip intersecting the box is selected.
      // mouseY values were recorded in viewport space; lift them into
      // logical (unscrolled) Y so we compare against each track's
      // y consistently regardless of the user's current scroll offset.
      const scrollOffset = getScrollOffset()
      const x1 = Math.min(drag.startMouseX, drag.currentMouseX)
      const x2 = Math.max(drag.startMouseX, drag.currentMouseX)
      const y1 = Math.min(drag.startMouseY, drag.currentMouseY) + verticalScroll
      const y2 = Math.max(drag.startMouseY, drag.currentMouseY) + verticalScroll
      const picked: string[] = []
      for (let i = 0; i < audioTracks.length; i++) {
        const y = RULER_HEIGHT + i * trackHeight
        if (y + trackHeight < y1 || y > y2) continue
        for (const clip of audioTracks[i].clips) {
          const cx = clip.position_ticks * pixelsPerTick - scrollOffset
          const cw = clip.length_ticks * pixelsPerTick
          if (cx + cw < x1 || cx > x2) continue
          picked.push(clip.id)
        }
      }
      if (picked.length > 0) {
        useTrackStore.setState(s => {
          const next = new Set(s.selectedClipIds)
          for (const id of picked) next.add(id)
          return { selectedClipIds: next, selectedClipId: picked[0] }
        })
      }
    } else if (drag && (drag.mode === 'move' || drag.mode === 'resize-right' || drag.mode === 'resize-left')) {
      // Flush the drag's optimistic local state to the backend in one
      // shot. We've been updating the store in-memory during mousemove
      // (moveClipLocal / resizeClipLocal) without IPC — now persist the
      // final position/length so save/load + audio engine see it.
      const liveTrack = useTrackStore.getState().tracks.find(t => t.id === drag.trackId)
      const liveClip = liveTrack?.clips?.find(c => c.id === drag.clipId)
      if (liveClip) {
        const lengthArg = drag.mode === 'move' ? undefined : liveClip.length_ticks
        commitClipDrag(drag.trackId, drag.clipId, liveClip.position_ticks, lengthArg)
          .catch(err => console.error('commitClipDrag failed', err))
      }
      // Group-move members each need their own commit too.
      if (drag.groupMoveOriginals) {
        for (const m of drag.groupMoveOriginals) {
          const t = useTrackStore.getState().tracks.find(tt => tt.id === m.trackId)
          const c = t?.clips?.find(cc => cc.id === m.clipId)
          if (c) {
            commitClipDrag(m.trackId, m.clipId, c.position_ticks)
              .catch(err => console.error('commitClipDrag (group) failed', err))
          }
        }
      }
    }
    dragRef.current = null
    forceRender(n => n + 1)
  }, [audioTracks, pixelsPerTick, trackHeight, getScrollOffset, commitClipDrag])

  const handleDoubleClick = useCallback((e: React.MouseEvent) => {
    const rect = (e.target as HTMLElement).getBoundingClientRect()
    const mouseX = e.clientX - rect.left
    const mouseY = e.clientY - rect.top
    if (mouseY < RULER_HEIGHT) return
    const hit = hitTest(mouseX, mouseY, getScrollOffset())
    if (!hit) return
    if (hit.clip.kind === 'midi') {
      useTrackStore.getState().setActiveMidiClip(hit.trackId, hit.clip.id)
      window.dispatchEvent(new CustomEvent('daw:openPianoRoll'))
    }
  }, [hitTest, getScrollOffset])

  const handleContextMenu = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
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
      setContextMenu(null)
      setMarkerCtx({ x: e.clientX, y: e.clientY, markerId: markerHit ? markerHit.id : null, tick: applySnap(tick) })
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
  }, [hitTest, getScrollOffset, selectClip, selectedClipIds, markers, pixelsPerTick, applySnap])

  const handleBrowserDragOver = useCallback((e: React.DragEvent) => {
    if (!e.dataTransfer.types.includes('application/x-hw-browser')) return
    e.preventDefault()
    e.dataTransfer.dropEffect = 'copy'
    setDropHighlight(true)
  }, [])

  const handleBrowserDragLeave = useCallback((e: React.DragEvent) => {
    const rt = e.relatedTarget as Node | null
    if (rt && containerRef.current?.contains(rt)) return
    setDropHighlight(false)
  }, [])

  const handleBrowserDrop = useCallback(async (e: React.DragEvent) => {
    const data = e.dataTransfer.getData('application/x-hw-browser')
    if (!data.startsWith('file:')) return
    e.preventDefault()
    setDropHighlight(false)
    const path = data.slice('file:'.length)
    const rect = containerRef.current?.getBoundingClientRect()
    if (!rect) return
    const mouseX = e.clientX - rect.left
    const mouseY = e.clientY - rect.top
    const scrollOffset = getScrollOffset()

    const state = useTrackStore.getState()
    const audio = state.tracks.filter(t => t.kind === 'Audio')
    const idx = Math.max(0, Math.floor((mouseY - RULER_HEIGHT) / trackHeight))
    const dropped = audioTracks[idx]
    let trackId: string | null = null
    if (dropped && dropped.kind === 'Audio') {
      trackId = dropped.id
    } else if (audio.length > 0) {
      const sel = state.selectedTrackId
      trackId = sel && audio.some(t => t.id === sel) ? sel : audio[0].id
    } else {
      await state.addAudioTrack()
      trackId = useTrackStore.getState().tracks.find(t => t.kind === 'Audio')?.id ?? null
    }
    if (!trackId) return

    const rawTicks = Math.max(0, Math.round((mouseX + scrollOffset) / pixelsPerTick))
    const positionTicks = applySnap(rawTicks)

    try {
      await useTrackStore.getState().importAudioFile(trackId, path, positionTicks)
      const { useBrowserStore } = await import('../../stores/browserStore')
      useBrowserStore.getState().pushFileRecent(path)
    } catch (err) {
      console.error('browser drop import failed:', path, err)
    }
  }, [audioTracks, pixelsPerTick, trackHeight, getScrollOffset, applySnap])

  const handleWheel = useCallback((e: React.WheelEvent) => {
    // Ctrl+Shift+Wheel: vertical zoom (change track height)
    if (e.ctrlKey && e.shiftKey) {
      e.preventDefault()
      const delta = e.deltaY < 0 ? 8 : -8
      setTrackHeight(trackHeight + delta)
      return
    }
    // Ctrl+Wheel: horizontal zoom
    if (e.ctrlKey) {
      e.preventDefault()
      const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15
      setHorizontalZoom(horizontalZoom * factor)
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
    const contentH = audioTracks.length * trackHeight
    const maxScroll = Math.max(0, contentH - viewportH)
    if (maxScroll <= 0) return
    // Normalize deltaY across mouse / trackpad / line-mode wheels —
    // pure line-mode (deltaMode=1) is the worst offender, sending ~3
    // for a single notch which feels frozen if applied 1:1.
    let dy = e.deltaY
    if (e.deltaMode === 1) dy *= 16
    else if (e.deltaMode === 2) dy *= viewportH
    setVerticalScroll((v) => Math.max(0, Math.min(maxScroll, v + dy)))
  }, [trackHeight, setTrackHeight, horizontalZoom, setHorizontalZoom, audioTracks.length])

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

  // Marker navigation hotkeys
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null
      if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) return
      const sr = sampleRate || 48000
      const playheadTicks = Math.round((positionSamples / sr) * (bpm / 60) * PPQ)
      if (e.altKey && e.code === 'ArrowRight') {
        e.preventDefault()
        const next = jumpToNext(playheadTicks)
        if (next) {
          const secs = next.tick / PPQ / (bpm / 60)
          setPosition(Math.round(secs * sr))
        }
      } else if (e.altKey && e.code === 'ArrowLeft') {
        e.preventDefault()
        const prev = jumpToPrev(playheadTicks)
        if (prev) {
          const secs = prev.tick / PPQ / (bpm / 60)
          setPosition(Math.round(secs * sr))
        }
      } else if (e.shiftKey && e.code === 'KeyM' && !e.ctrlKey && !e.metaKey) {
        e.preventDefault()
        addMarker(playheadTicks)
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [positionSamples, sampleRate, bpm, jumpToNext, jumpToPrev, addMarker, setPosition])

  // Drag-and-drop
  const [dropHighlight, setDropHighlight] = useState(false)

  useEffect(() => {
    const audioExts = ['wav', 'flac', 'mp3', 'ogg', 'aac', 'm4a', 'aiff', 'aif', 'aifc']
    const { importAudioFile, addAudioTrack } = useTrackStore.getState()
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
    const handleDropPayload = async (allPaths: string[]) => {
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
        pushNotif('warning', 'Dropped file is not a supported audio format', {
          detail: `Supported: ${audioExts.join(', ')}`,
        })
        return
      }
      if (files.length === 0) {
        log({ level: 'event', test: 'drag-drop', message: 'no paths in payload (empty drop)' })
        return
      }

      const state = useTrackStore.getState()
      // Clip placement target: prefer a selected insert, else fall back
      // to the first pre-allocated insert (insert-001). The 500-insert
      // architecture means there's always at least one insert ready —
      // the only reason addAudioTrack runs is the legacy edge case
      // where a project was loaded without inserts.
      let trackId: string | null = state.selectedTrackId
      const inserts = state.tracks.filter(t => t.id.startsWith('insert-'))
      if (!trackId || !inserts.find(t => t.id === trackId)) {
        if (inserts.length === 0) {
          trackId = await addAudioTrack()
        } else {
          trackId = inserts[0].id
        }
      }
      if (!trackId) {
        pushNotif('error', 'Could not find or create an audio track for dropped file')
        return
      }

      let offsetTicks = 0
      const track = useTrackStore.getState().tracks.find(t => t.id === trackId)
      if (track) {
        for (const clip of track.clips) {
          const end = clip.position_ticks + clip.length_ticks
          if (end > offsetTicks) offsetTicks = end
        }
      }

      const { useBrowserStore } = await import('../../stores/browserStore')
      const pushRecent = useBrowserStore.getState().pushFileRecent
      let imported = 0
      for (const file of files) {
        try {
          // Channel Rack entry per UNIQUE sample (FL Studio model).
          // Dropping kick.wav three times must produce one "kick"
          // channel and three playlist clips, not three identical
          // channels. We dedupe on the basename: if a non-insert
          // audio track named after this sample already exists, skip
          // the addAudioTrack call and let the clip on the insert
          // share the existing channel's identity.
          const sampleName =
            file
              .split(/[\\/]/)
              .pop()
              ?.replace(/\.[^.]+$/, '') || 'Sample'
          const existingChannel = useTrackStore
            .getState()
            .tracks.find(
              t =>
                t.kind === 'Audio' &&
                !t.id.startsWith('insert-') &&
                t.name === sampleName,
            )
          if (!existingChannel) {
            await addAudioTrack(sampleName)
          }

          const result = await importAudioFile(trackId, file, offsetTicks)
          offsetTicks += result.length_ticks
          pushRecent(file)
          imported++
          log({ level: 'pass', test: 'drag-drop', message: `imported ${file.split(/[\\/]/).pop() || file}`, actual: `+${result.length_ticks} ticks` })
        } catch (e) {
          console.error('Failed to import:', file, e)
          log({ level: 'fail', test: 'drag-drop', message: `import failed: ${file.split(/[\\/]/).pop() || file}`, actual: String(e) })
          pushNotif('error', `Failed to import ${file.split(/[\\/]/).pop()}`, { detail: String(e) })
        }
      }
      if (imported > 0) {
        log({ level: 'info', test: 'drag-drop', message: `imported ${imported}/${files.length} file(s) on track ${trackId}` })
        pushNotif('info', imported === files.length
          ? `Imported ${imported} file${imported === 1 ? '' : 's'}`
          : `Imported ${imported} of ${files.length} files`)
      }
    }

    // De-dupe: if both channels fire for the same drop within 200 ms,
    // ignore the second one. Windows users will likely receive the
    // native `daw:drag-drop` first; macOS / Linux users get the
    // standard `tauri://drag-drop` channel.
    let lastDropAt = 0
    const dispatchDrop = (paths: string[]) => {
      const now = Date.now()
      if (now - lastDropAt < 200) return
      lastDropAt = now
      void handleDropPayload(paths)
    }

    const unlistenDrop = listen<{ paths: string[] }>('tauri://drag-drop', (event) => {
      log({ level: 'event', test: 'drag-drop', message: 'channel:tauri://drag-drop fired', actual: `${event.payload?.paths?.length ?? 0} path(s)` })
      dispatchDrop(event.payload?.paths ?? [])
    })
    const unlistenNativeDrop = listen<{ kind: string; paths?: string[] }>(
      'daw:drag-drop',
      (event) => {
        const payload = event.payload
        if (!payload) return
        if (payload.kind === 'enter' || payload.kind === 'over') setDropHighlight(true)
        else if (payload.kind === 'leave') setDropHighlight(false)
        else if (payload.kind === 'drop') {
          log({ level: 'event', test: 'drag-drop', message: 'channel:daw:drag-drop (native) fired', actual: `${payload.paths?.length ?? 0} path(s)` })
          dispatchDrop(payload.paths ?? [])
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

  return (
    <div
      ref={containerRef}
      onDragOver={handleBrowserDragOver}
      onDragLeave={handleBrowserDragLeave}
      onDrop={handleBrowserDrop}
      style={{
        flex: 1,
        position: 'relative',
        overflow: 'hidden',
        // Background intentionally absent — the parent .fl-pl-grid renders
        // the mockup's purple gradient (#1a0e1f → #0e0716) and the canvas
        // is now transparent so it shows through.
        ...(dropHighlight ? { outline: `2px solid ${hw.accent}`, outlineOffset: -2 } : {}),
      }}
    >
      <canvas
        ref={canvasRef}
        data-testid="arrangement-canvas"
        data-track-height={trackHeight}
        style={{ position: 'absolute', top: 0, left: 0 }}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
        onDoubleClick={handleDoubleClick}
        onContextMenu={handleContextMenu}
        onWheel={handleWheel}
      />
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
          <MenuItem label="Duplicate (Ctrl+D)" onClick={async () => {
            await duplicateClip(contextMenu.trackId, contextMenu.clipId)
            setContextMenu(null)
          }} />
          <MenuItem label="Split at playhead (S)" onClick={async () => {
            const sr = sampleRate || 48000
            const atTicks = Math.round((positionSamples / sr) * (bpm / 60) * 960)
            try { await splitClip(contextMenu.trackId, contextMenu.clipId, atTicks) } catch {}
            setContextMenu(null)
          }} />
          <MenuItem label={menuClip?.reversed ? 'Un-reverse' : 'Reverse audio'} onClick={async () => {
            await toggleClipReverse(contextMenu.trackId, contextMenu.clipId)
            setContextMenu(null)
          }} />
          <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
          {(() => {
            const gid = clipToGroup[contextMenu.clipId]
            const selectionIds = Array.from(selectedClipIds)
            const canGroup = !gid && selectionIds.length >= 2 && selectionIds.includes(contextMenu.clipId)
            return (
              <>
                {canGroup && (
                  <MenuItem label={`Group ${selectionIds.length} clips`} onClick={() => {
                    groupClipsAction(selectionIds)
                    setContextMenu(null)
                  }} />
                )}
                {!gid && !canGroup && (
                  <MenuItem label="Group (select 2+ clips first)" onClick={() => setContextMenu(null)} />
                )}
                {gid && (
                  <MenuItem label="Ungroup this clip" onClick={() => {
                    ungroupClipAction(contextMenu.clipId)
                    setContextMenu(null)
                  }} />
                )}
              </>
            )
          })()}
          <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
          <MenuItem label="Reset gain (0 dB)" onClick={async () => {
            await setClipGain(contextMenu.trackId, contextMenu.clipId, 0)
            setContextMenu(null)
          }} />
          <MenuItem label="Reset fades" onClick={async () => {
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
                  Fade in curve
                </div>
                {curves.map(c => (
                  <MenuItem key={`fi_${c.value}`} label={`${c.value === fi ? '● ' : '   '}${c.label}`}
                    onClick={async () => {
                      await setClipFadeCurves(contextMenu.trackId, contextMenu.clipId, c.value, fo)
                      setContextMenu(null)
                    }} />
                ))}
                <div style={{ padding: '4px 10px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
                  Fade out curve
                </div>
                {curves.map(c => (
                  <MenuItem key={`fo_${c.value}`} label={`${c.value === fo ? '● ' : '   '}${c.label}`}
                    onClick={async () => {
                      await setClipFadeCurves(contextMenu.trackId, contextMenu.clipId, fi, c.value)
                      setContextMenu(null)
                    }} />
                ))}
              </>
            )
          })()}
          <MenuItem label="Reset pitch (0 st)" onClick={async () => {
            await setClipPitch(contextMenu.trackId, contextMenu.clipId, 0)
            setContextMenu(null)
          }} />
          <MenuItem label="Reset stretch (1.00×)" onClick={async () => {
            await setClipStretch(contextMenu.trackId, contextMenu.clipId, 1)
            setContextMenu(null)
          }} />
          <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
          <div style={{ padding: '4px 10px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
            Color
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(8, 1fr)', gap: 2, padding: '2px 6px 4px' }}>
            {CLIP_COLORS.map(c => {
              const active = clipColorOverrides[contextMenu.clipId] === c
              return (
                <button key={c} title={c}
                  onClick={() => { setClipColor(contextMenu.clipId, c); setContextMenu(null) }}
                  style={{
                    width: 18, height: 18, borderRadius: 3, background: c,
                    border: active ? '2px solid #fff' : '1px solid rgba(255,255,255,0.12)',
                    cursor: 'pointer', padding: 0,
                  }}
                />
              )
            })}
          </div>
          <MenuItem label="Clear color override" onClick={() => {
            setClipColor(contextMenu.clipId, null)
            setContextMenu(null)
          }} />
          <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
          <MenuItem label="Delete" danger onClick={async () => {
            await deleteClip(contextMenu.trackId, contextMenu.clipId)
            setContextMenu(null)
          }} />
        </div>
        )
      })()}
      {markerCtx && (() => {
        const hit = markerCtx.markerId ? markers.find(m => m.id === markerCtx.markerId) : null
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
              {hit ? hit.label : `Tick ${markerCtx.tick}`}
            </div>
            {!hit && (
              <>
                <MenuItem label="Add marker here" onClick={() => {
                  addMarker(markerCtx.tick)
                  setMarkerCtx(null)
                }} />
                <MenuItem label="Add tempo change…" onClick={() => {
                  const raw = window.prompt('Tempo (BPM):', String(bpm))
                  if (raw == null) { setMarkerCtx(null); return }
                  const parsed = parseFloat(raw)
                  if (Number.isFinite(parsed) && parsed > 0) addTempoMarker(markerCtx.tick, parsed)
                  setMarkerCtx(null)
                }} />
                <MenuItem label="Add time signature change…" onClick={() => {
                  const raw = window.prompt('Time signature (e.g. 4/4, 6/8, 7/8):', '4/4')
                  if (raw == null) { setMarkerCtx(null); return }
                  const m = raw.trim().match(/^(\d+)\s*\/\s*(\d+)$/)
                  if (m) addTimeSigMarker(markerCtx.tick, parseInt(m[1], 10), parseInt(m[2], 10))
                  setMarkerCtx(null)
                }} />
                <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
                <MenuItem label="Set punch-in here" onClick={() => {
                  setPunchIn(markerCtx.tick)
                  if (punchOutTicks != null && punchOutTicks <= markerCtx.tick) setPunchOut(null)
                  setMarkerCtx(null)
                }} />
                <MenuItem label="Set punch-out here" onClick={() => {
                  setPunchOut(markerCtx.tick)
                  if (punchInTicks != null && punchInTicks >= markerCtx.tick) setPunchIn(null)
                  setMarkerCtx(null)
                }} />
                {looping && loopEnd > loopStart && (
                  <MenuItem label="Set punch range from loop" onClick={() => {
                    setPunchRangeFromLoop()
                    setMarkerCtx(null)
                  }} />
                )}
                {(punchInTicks != null || punchOutTicks != null) && (
                  <MenuItem label="Clear punch range" onClick={() => {
                    clearPunch()
                    setMarkerCtx(null)
                  }} />
                )}
              </>
            )}
            {hit && (
              <>
                {hit.kind === 'tempo' && (
                  <MenuItem label="Edit tempo…" onClick={() => {
                    const raw = window.prompt('Tempo (BPM):', String(hit.bpm ?? bpm))
                    if (raw == null) { setMarkerCtx(null); return }
                    const parsed = parseFloat(raw)
                    if (Number.isFinite(parsed) && parsed > 0) {
                      const clamped = Math.max(20, Math.min(999, parsed))
                      updateMarker(hit.id, {
                        bpm: clamped,
                        label: `${clamped.toFixed(clamped % 1 === 0 ? 0 : 2)} BPM`,
                      })
                    }
                    setMarkerCtx(null)
                  }} />
                )}
                {hit.kind === 'timesig' && (
                  <MenuItem label="Edit time signature…" onClick={() => {
                    const current = `${hit.timeSigNum ?? 4}/${hit.timeSigDen ?? 4}`
                    const raw = window.prompt('Time signature (e.g. 4/4, 6/8):', current)
                    if (raw == null) { setMarkerCtx(null); return }
                    const m = raw.trim().match(/^(\d+)\s*\/\s*(\d+)$/)
                    if (m) {
                      const n = Math.max(1, Math.min(32, parseInt(m[1], 10)))
                      const dRaw = parseInt(m[2], 10)
                      const d = [1, 2, 4, 8, 16, 32].includes(dRaw) ? dRaw : 4
                      updateMarker(hit.id, { timeSigNum: n, timeSigDen: d, label: `${n}/${d}` })
                    }
                    setMarkerCtx(null)
                  }} />
                )}
                {(!hit.kind || hit.kind === 'generic') && (
                  <MenuItem label="Rename" onClick={() => {
                    setRenamingMarker({ id: hit.id, draft: hit.label })
                    setMarkerCtx(null)
                  }} />
                )}
                {(!hit.kind || hit.kind === 'generic') && (
                  <>
                    <div style={{ height: 1, background: hw.border, margin: '3px 4px' }} />
                    <div style={{ padding: '4px 10px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
                      Color
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
                <MenuItem label="Delete marker" danger onClick={() => {
                  removeMarker(hit.id)
                  setMarkerCtx(null)
                }} />
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
          Drop audio files here or add tracks from the toolbar
        </div>
      )}
    </div>
  )
}

function MenuItem({ label, onClick, danger }: { label: string; onClick: () => void; danger?: boolean }) {
  return (
    <button
      onClick={onClick}
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

function darkenColor(hex: string, amount: number): string {
  const r = Math.round(parseInt(hex.slice(1, 3), 16) * (1 - amount))
  const g = Math.round(parseInt(hex.slice(3, 5), 16) * (1 - amount))
  const b = Math.round(parseInt(hex.slice(5, 7), 16) * (1 - amount))
  return `rgb(${r},${g},${b})`
}

function lightenColor(hex: string, amount: number): string {
  const r = Math.min(255, Math.round(parseInt(hex.slice(1, 3), 16) * (1 + amount)))
  const g = Math.min(255, Math.round(parseInt(hex.slice(3, 5), 16) * (1 + amount)))
  const b = Math.min(255, Math.round(parseInt(hex.slice(5, 7), 16) * (1 + amount)))
  return `rgb(${r},${g},${b})`
}
