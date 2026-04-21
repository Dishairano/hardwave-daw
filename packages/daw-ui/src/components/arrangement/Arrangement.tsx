import { useRef, useEffect, useState, useCallback } from 'react'
import { listen } from '@tauri-apps/api/event'
import { useTrackStore, ClipInfo } from '../../stores/trackStore'
import { useTransportStore, snapToTicks } from '../../stores/transportStore'
import { hw } from '../../theme'

const PPQ = 960
const PIXELS_PER_SECOND_BASE = 100
const RESIZE_HANDLE_PX = 6
const RULER_HEIGHT = 22

type DragMode = 'none' | 'move' | 'resize-right' | 'resize-left' | 'fade-in' | 'fade-out' | 'rubber' | 'scrub'

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

export function Arrangement() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<DragState | null>(null)
  const [, forceRender] = useState(0)

  const {
    tracks, selectedClipId, selectedClipIds, selectClip, toggleClipSelection, clearSelection,
    moveClip, resizeClip, getWaveformPeaks, duplicateClip, splitClip, deleteClip, setClipFades,
  } = useTrackStore()
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null)
  const {
    positionSamples, playing, bpm, sampleRate, setPosition, looping, loopStart, loopEnd,
    trackHeight, setTrackHeight, snapValue, snapEnabled, horizontalZoom, setHorizontalZoom,
    clipColorOverrides, editCursorTicks, setEditCursor,
  } = useTransportStore()

  const audioTracks = tracks.filter(t => t.kind !== 'Master')
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

    // Background — near-black
    ctx.fillStyle = '#0a0a0f'
    ctx.fillRect(0, 0, w, h)

    // Ruler bar
    ctx.fillStyle = '#08080d'
    ctx.fillRect(0, 0, w, RULER_HEIGHT)
    ctx.strokeStyle = 'rgba(255,255,255,0.04)'
    ctx.lineWidth = 1
    ctx.beginPath()
    ctx.moveTo(0, RULER_HEIGHT)
    ctx.lineTo(w, RULER_HEIGHT)
    ctx.stroke()

    // Beat grid + snap subdivisions
    const startBeat = Math.floor(scrollOffset / pixelsPerBeat)
    for (let i = startBeat; i < startBeat + Math.ceil(w / pixelsPerBeat) + 2; i++) {
      const x = i * pixelsPerBeat - scrollOffset
      if (x < -1 || x > w + 1) continue

      const isBar = i % 4 === 0

      ctx.strokeStyle = isBar ? 'rgba(255,255,255,0.06)' : 'rgba(255,255,255,0.02)'
      ctx.lineWidth = isBar ? 1 : 0.5
      ctx.beginPath()
      ctx.moveTo(x, RULER_HEIGHT)
      ctx.lineTo(x, h)
      ctx.stroke()

      // Ruler markings
      if (isBar) {
        ctx.strokeStyle = 'rgba(255,255,255,0.06)'
        ctx.beginPath()
        ctx.moveTo(x, 0)
        ctx.lineTo(x, RULER_HEIGHT)
        ctx.stroke()

        ctx.fillStyle = '#71717a'
        ctx.font = '9px Inter, ui-sans-serif, sans-serif'
        ctx.fillText(`${Math.floor(i / 4) + 1}`, x + 3, 13)
      } else {
        ctx.strokeStyle = 'rgba(255,255,255,0.03)'
        ctx.beginPath()
        ctx.moveTo(x, RULER_HEIGHT - 4)
        ctx.lineTo(x, RULER_HEIGHT)
        ctx.stroke()
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

    // Track lane backgrounds
    for (let i = 0; i < audioTracks.length; i++) {
      const y = RULER_HEIGHT + i * trackHeight
      ctx.fillStyle = i % 2 === 0 ? '#0a0a0f' : '#0c0c11'
      ctx.fillRect(0, y, w, trackHeight)

      ctx.strokeStyle = 'rgba(255,255,255,0.03)'
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(0, y + trackHeight)
      ctx.lineTo(w, y + trackHeight)
      ctx.stroke()
    }

    // Clips
    for (let i = 0; i < audioTracks.length; i++) {
      const y = RULER_HEIGHT + i * trackHeight
      const track = audioTracks[i]
      const defaultColor = CLIP_COLORS[i % CLIP_COLORS.length]

      for (const clip of track.clips) {
        const color = clipColorOverrides[clip.id] || defaultColor
        drawClip(ctx, clip, color, y, scrollOffset, w, pixelsPerTick)
      }
    }

    // Crossfade overlay — shade any overlap between adjacent clips on the same track.
    for (let i = 0; i < audioTracks.length; i++) {
      const y = RULER_HEIGHT + i * trackHeight
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

    // Playhead — red
    const playheadX = playheadSecs * PIXELS_PER_SECOND - scrollOffset
    if (playing || positionSamples > 0) {
      ctx.strokeStyle = '#DC2626'
      ctx.lineWidth = 1.5
      ctx.beginPath()
      ctx.moveTo(playheadX, 0)
      ctx.lineTo(playheadX, h)
      ctx.stroke()

      // Glow
      ctx.strokeStyle = 'rgba(220,38,38,0.15)'
      ctx.lineWidth = 6
      ctx.beginPath()
      ctx.moveTo(playheadX, 0)
      ctx.lineTo(playheadX, h)
      ctx.stroke()

      ctx.fillStyle = '#DC2626'
      ctx.beginPath()
      ctx.moveTo(playheadX - 5, 0)
      ctx.lineTo(playheadX + 5, 0)
      ctx.lineTo(playheadX, 7)
      ctx.closePath()
      ctx.fill()
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

  }, [tracks, positionSamples, playing, bpm, sampleRate, selectedClipId, selectedClipIds, looping, loopStart, loopEnd, trackHeight, horizontalZoom, snapValue, snapEnabled, clipColorOverrides, editCursorTicks])

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

    const pad = 2
    const x = clipX
    const y = trackY + pad
    const w = clipW
    const h = trackHeight - pad * 2
    const isSelected = clip.id === selectedClipId || selectedClipIds.has(clip.id)
    const color = clip.muted ? '#1a1a24' : baseColor

    // Clip body
    ctx.fillStyle = color
    ctx.globalAlpha = clip.muted ? 0.3 : 0.25
    ctx.beginPath()
    ctx.roundRect(x, y, w, h, 6)
    ctx.fill()
    ctx.globalAlpha = 1.0

    // Clip header bar
    const headerH = 14
    ctx.fillStyle = clip.muted ? '#161620' : darkenColor(color, 0.4)
    ctx.globalAlpha = clip.muted ? 0.5 : 0.8
    ctx.beginPath()
    ctx.roundRect(x, y, w, headerH, [6, 6, 0, 0])
    ctx.fill()
    ctx.globalAlpha = 1

    // Clip name
    ctx.fillStyle = clip.muted ? '#52525b' : '#EEE'
    ctx.font = '9px Inter, ui-sans-serif, sans-serif'
    ctx.save()
    ctx.beginPath()
    ctx.rect(x + 2, y, w - 4, headerH)
    ctx.clip()
    ctx.fillText(clip.name, x + 4, y + 10)
    ctx.restore()

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
  }

  // Hit test
  const hitTest = useCallback((mouseX: number, mouseY: number, scrollOffset: number): {
    clip: ClipInfo, trackId: string, edge: 'body' | 'left' | 'right' | 'fade-in' | 'fade-out'
  } | null => {
    for (let i = 0; i < audioTracks.length; i++) {
      const y = RULER_HEIGHT + i * trackHeight
      if (mouseY < y + 2 || mouseY > y + trackHeight - 2) continue
      const track = audioTracks[i]
      for (const clip of track.clips) {
        const clipX = clip.position_ticks * pixelsPerTick - scrollOffset
        const clipW = clip.length_ticks * pixelsPerTick
        if (mouseX < clipX || mouseX > clipX + clipW) continue
        const localY = mouseY - (y + 2)
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
  }, [audioTracks, pixelsPerTick, trackHeight])

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
      if (e.ctrlKey || e.metaKey) {
        toggleClipSelection(hit.clip.id)
      } else if (!selectedClipIds.has(hit.clip.id)) {
        selectClip(hit.clip.id, hit.trackId)
      } else {
        selectClip(hit.clip.id, hit.trackId)
      }
      const mode: DragMode =
        hit.edge === 'right' ? 'resize-right' :
        hit.edge === 'left' ? 'resize-left' :
        hit.edge === 'fade-in' ? 'fade-in' :
        hit.edge === 'fade-out' ? 'fade-out' :
        'move'
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
      }
    } else {
      if (!(e.ctrlKey || e.metaKey)) clearSelection()
      // Place the edit cursor at the clicked tick (snapped). It may be replaced by a
      // rubber-band selection if the user drags.
      const tickAt = Math.max(0, Math.round((mouseX + scrollOffset) / pixelsPerTick))
      setEditCursor(applySnap(tickAt))
      dragRef.current = {
        mode: 'rubber', clipId: '', trackId: '',
        startMouseX: mouseX, startMouseY: mouseY, currentMouseX: mouseX, currentMouseY: mouseY,
        originalPositionTicks: 0, originalLengthTicks: 0,
        originalFadeInTicks: 0, originalFadeOutTicks: 0,
      }
      forceRender(n => n + 1)
    }
  }, [hitTest, selectClip, toggleClipSelection, clearSelection, selectedClipIds, getScrollOffset, PIXELS_PER_SECOND, sampleRate, setPosition, pixelsPerTick, setEditCursor, snapTicks])

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
      const newPos = Math.max(0, drag.originalPositionTicks + dTicks)
      moveClip(drag.trackId, drag.clipId, applySnap(newPos))
    } else if (drag.mode === 'resize-right') {
      const newLen = Math.max(minLen, drag.originalLengthTicks + dTicks)
      resizeClip(drag.trackId, drag.clipId, applySnap(newLen))
    } else if (drag.mode === 'resize-left') {
      // Keep the right edge anchored; shift position and shrink length.
      const rightEdge = drag.originalPositionTicks + drag.originalLengthTicks
      let newPos = Math.max(0, drag.originalPositionTicks + dTicks)
      newPos = applySnap(newPos)
      if (newPos > rightEdge - minLen) newPos = rightEdge - minLen
      const newLen = rightEdge - newPos
      moveClip(drag.trackId, drag.clipId, newPos)
      resizeClip(drag.trackId, drag.clipId, newLen)
    }
  }, [hitTest, getScrollOffset, pixelsPerTick, moveClip, resizeClip, setClipFades, snapTicks, PIXELS_PER_SECOND, sampleRate, setPosition])

  const handleMouseUp = useCallback(() => {
    const drag = dragRef.current
    if (drag && drag.mode === 'rubber') {
      // Finalize rubber-band selection — any clip intersecting the box is selected.
      const scrollOffset = getScrollOffset()
      const x1 = Math.min(drag.startMouseX, drag.currentMouseX)
      const x2 = Math.max(drag.startMouseX, drag.currentMouseX)
      const y1 = Math.min(drag.startMouseY, drag.currentMouseY)
      const y2 = Math.max(drag.startMouseY, drag.currentMouseY)
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
    }
    dragRef.current = null
    forceRender(n => n + 1)
  }, [audioTracks, pixelsPerTick, trackHeight, getScrollOffset])

  const handleContextMenu = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    const rect = (e.target as HTMLElement).getBoundingClientRect()
    const mouseX = e.clientX - rect.left
    const mouseY = e.clientY - rect.top
    const hit = hitTest(mouseX, mouseY, getScrollOffset())
    if (hit) {
      if (!selectedClipIds.has(hit.clip.id)) {
        selectClip(hit.clip.id, hit.trackId)
      }
      setContextMenu({ x: e.clientX, y: e.clientY, trackId: hit.trackId, clipId: hit.clip.id })
    } else {
      setContextMenu(null)
    }
  }, [hitTest, getScrollOffset, selectClip, selectedClipIds])

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
    }
  }, [trackHeight, setTrackHeight, horizontalZoom, setHorizontalZoom])

  // Drag-and-drop
  const [dropHighlight, setDropHighlight] = useState(false)

  useEffect(() => {
    const audioExts = ['wav', 'flac', 'mp3', 'ogg', 'aac', 'm4a']
    const { importAudioFile, addAudioTrack } = useTrackStore.getState()

    const unlistenDrop = listen<{ paths: string[] }>('tauri://drag-drop', async (event) => {
      setDropHighlight(false)
      const files = event.payload.paths.filter(p => {
        const ext = p.split('.').pop()?.toLowerCase() || ''
        return audioExts.includes(ext)
      })
      if (files.length === 0) return

      const state = useTrackStore.getState()
      let trackId = state.selectedTrackId
      const audio = state.tracks.filter(t => t.kind === 'Audio')
      if (!trackId || !audio.find(t => t.id === trackId)) {
        if (audio.length === 0) {
          await addAudioTrack()
          trackId = useTrackStore.getState().tracks.find(t => t.kind === 'Audio')?.id || null
        } else {
          trackId = audio[0].id
        }
      }
      if (!trackId) return

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
      for (const file of files) {
        try {
          const result = await importAudioFile(trackId, file, offsetTicks)
          offsetTicks += result.length_ticks
          pushRecent(file)
        } catch (e) {
          console.error('Failed to import:', file, e)
        }
      }
    })

    const unlistenOver = listen('tauri://drag-over', () => setDropHighlight(true))
    const unlistenLeave = listen('tauri://drag-leave', () => setDropHighlight(false))

    return () => {
      unlistenDrop.then(f => f())
      unlistenOver.then(f => f())
      unlistenLeave.then(f => f())
    }
  }, [])

  return (
    <div
      ref={containerRef}
      style={{
        flex: 1,
        position: 'relative',
        overflow: 'hidden',
        background: '#0a0a0f',
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
      {contextMenu && (
        <div
          data-testid="clip-context-menu"
          onMouseLeave={() => setContextMenu(null)}
          style={{
            position: 'fixed', left: contextMenu.x, top: contextMenu.y,
            background: '#12121a', border: `1px solid ${hw.border}`,
            borderRadius: 6, padding: 4, minWidth: 160, zIndex: 1000,
            boxShadow: '0 8px 24px rgba(0,0,0,0.6)',
          }}
        >
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
          <MenuItem label="Delete" danger onClick={async () => {
            await deleteClip(contextMenu.trackId, contextMenu.clipId)
            setContextMenu(null)
          }} />
        </div>
      )}
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
