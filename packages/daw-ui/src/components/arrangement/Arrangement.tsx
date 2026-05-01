import { useRef, useEffect, useState, useCallback } from 'react'
import { listen } from '@tauri-apps/api/event'
import { useTrackStore, ClipInfo } from '../../stores/trackStore'
import { useTransportStore } from '../../stores/transportStore'
import { hw } from '../../theme'

const PPQ = 960
// Mockup-aligned defaults: compact 18px rows like the FL-style playlist density.
const TRACK_HEIGHT_DEFAULT = 18
const PIXELS_PER_SECOND_DEFAULT = 100
const RESIZE_HANDLE_PX = 4
const RULER_HEIGHT = 28

// Zoom limits — per user spec, vertical can't shrink below 0.6×
const PPS_MIN = 30
const PPS_MAX = 600
const TRACK_H_MIN = Math.max(12, Math.round(TRACK_HEIGHT_DEFAULT * 0.6))
const TRACK_H_MAX = TRACK_HEIGHT_DEFAULT * 4 // allow zooming to thicker rows

type DragMode = 'none' | 'move' | 'resize-right'

interface DragState {
  mode: DragMode
  clipId: string
  trackId: string
  startMouseX: number
  originalPositionTicks: number
  originalLengthTicks: number
  altBypassedSnap: boolean
}

const waveformData = new Map<string, [number, number][]>()

// Hardwave clip palette — vibrant on near-black
const CLIP_COLORS = [
  '#DC2626', '#10B981', '#A855F7', '#F59E0B',
  '#3B82F6', '#EC4899', '#06B6D4', '#84CC16',
]

const clamp = (n: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, n))

interface ArrangementProps {
  onSetHint?: (text: string) => void
}

export function Arrangement({ onSetHint }: ArrangementProps = {}) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<DragState | null>(null)
  const [, forceRender] = useState(0)

  const { tracks, selectedClipId, selectClip, moveClip, resizeClip, getWaveformPeaks } = useTrackStore()
  const { positionSamples, playing, bpm, sampleRate } = useTransportStore()

  // Two-axis zoom + scroll state. autoFollow follows the playhead until the user pans manually.
  const [pps, setPps] = useState(PIXELS_PER_SECOND_DEFAULT)
  const [trackH, setTrackH] = useState(TRACK_HEIGHT_DEFAULT)
  const [scrollY, setScrollY] = useState(0)
  const [autoFollow, setAutoFollow] = useState(true)

  const ppsRef = useRef(pps); ppsRef.current = pps
  const trackHRef = useRef(trackH); trackHRef.current = trackH
  const scrollYRef = useRef(scrollY); scrollYRef.current = scrollY
  const autoFollowRef = useRef(autoFollow); autoFollowRef.current = autoFollow

  const audioTracks = tracks.filter(t => t.kind !== 'Master')
  const beatsPerSecond = bpm / 60
  const pixelsPerBeat = pps / beatsPerSecond
  const pixelsPerTick = pixelsPerBeat / PPQ
  // Aliases so the rest of the draw/hit-test code reads cleanly with the new state-driven values.
  const TRACK_HEIGHT = trackH
  const PIXELS_PER_SECOND = pps

  // Load waveforms
  useEffect(() => {
    for (const track of audioTracks) {
      for (const clip of track.clips) {
        if (clip.source_id && !waveformData.has(clip.source_id)) {
          const numBuckets = Math.max(100, Math.ceil(clip.length_ticks * pixelsPerTick / 2))
          getWaveformPeaks(clip.source_id, Math.min(numBuckets, 2000)).then(peaks => {
            waveformData.set(clip.source_id, peaks)
            forceRender(n => n + 1)
          })
        }
      }
    }
  }, [tracks])

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
    const scrollOffset = autoFollow
      ? Math.max(0, playheadSecs * PIXELS_PER_SECOND - w * 0.25)
      : 0

    // Background — pure black to match mockup
    ctx.fillStyle = '#000'
    ctx.fillRect(0, 0, w, h)

    // Ruler bar (always pinned at top, never scrolls vertically)
    ctx.fillStyle = '#040406'
    ctx.fillRect(0, 0, w, RULER_HEIGHT)
    ctx.strokeStyle = 'rgba(255,255,255,0.06)'
    ctx.lineWidth = 1
    ctx.beginPath()
    ctx.moveTo(0, RULER_HEIGHT + 0.5)
    ctx.lineTo(w, RULER_HEIGHT + 0.5)
    ctx.stroke()

    // Beat grid — HQ pixel-snapped 1px hairlines (use Math.floor + 0.5 for crisp lines)
    const startBeat = Math.floor(scrollOffset / pixelsPerBeat)
    for (let i = startBeat; i < startBeat + Math.ceil(w / pixelsPerBeat) + 2; i++) {
      const xRaw = i * pixelsPerBeat - scrollOffset
      const x = Math.floor(xRaw) + 0.5
      if (x < -1 || x > w + 1) continue

      const isBar = i % 4 === 0

      ctx.strokeStyle = isBar ? 'rgba(255,255,255,0.11)' : 'rgba(255,255,255,0.04)'
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(x, RULER_HEIGHT)
      ctx.lineTo(x, h)
      ctx.stroke()

      // Ruler markings
      if (isBar) {
        ctx.strokeStyle = 'rgba(255,255,255,0.12)'
        ctx.beginPath()
        ctx.moveTo(x, 0)
        ctx.lineTo(x, RULER_HEIGHT)
        ctx.stroke()

        ctx.fillStyle = '#a1a1aa'
        ctx.font = "8px 'JetBrains Mono', ui-monospace, Menlo, monospace"
        ctx.fillText(`${Math.floor(i / 4) + 1}`, Math.floor(xRaw) + 3, 16)
      } else {
        ctx.strokeStyle = 'rgba(255,255,255,0.05)'
        ctx.beginPath()
        ctx.moveTo(x, RULER_HEIGHT - 4)
        ctx.lineTo(x, RULER_HEIGHT)
        ctx.stroke()
      }
    }

    // Track lane backgrounds (apply vertical scroll, clip below ruler)
    ctx.save()
    ctx.beginPath()
    ctx.rect(0, RULER_HEIGHT, w, h - RULER_HEIGHT)
    ctx.clip()

    for (let i = 0; i < audioTracks.length; i++) {
      const y = Math.floor(RULER_HEIGHT + i * TRACK_HEIGHT - scrollY) + 0.5
      if (y > h || y + TRACK_HEIGHT < RULER_HEIGHT) continue
      // Subtle alternating row stripe — slightly darker in mockup palette
      ctx.fillStyle = i % 2 === 0 ? '#08080c' : '#0b0b10'
      ctx.fillRect(0, y - 0.5, w, TRACK_HEIGHT)

      // Bottom border — darker to match mockup
      ctx.strokeStyle = '#0a0a0e'
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(0, y + TRACK_HEIGHT - 0.5)
      ctx.lineTo(w, y + TRACK_HEIGHT - 0.5)
      ctx.stroke()
    }

    // Clips (within the same vertical-scroll clip region)
    for (let i = 0; i < audioTracks.length; i++) {
      const y = RULER_HEIGHT + i * TRACK_HEIGHT - scrollY
      if (y > h || y + TRACK_HEIGHT < RULER_HEIGHT) continue
      const track = audioTracks[i]
      const fallbackColor = CLIP_COLORS[i % CLIP_COLORS.length]

      for (const clip of track.clips) {
        // Mockup: each clip can carry its own --col override; otherwise inherit from track index.
        const clipColor = clip.color || track.color || fallbackColor
        drawClip(ctx, clip, clipColor, y, scrollOffset, w, pixelsPerTick)
      }
    }
    ctx.restore()

    // Playhead — bright red, sharper 1px stroke with stronger glow
    const playheadX = Math.floor(playheadSecs * PIXELS_PER_SECOND - scrollOffset) + 0.5
    if (playing || positionSamples > 0) {
      // Glow first (so the sharp 1px line draws on top)
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

  }, [tracks, positionSamples, playing, bpm, sampleRate, selectedClipId, pps, trackH, scrollY, autoFollow])

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
    const h = TRACK_HEIGHT - pad * 2
    const isSelected = clip.id === selectedClipId
    const color = clip.muted ? '#1a1a24' : baseColor
    // Mockup uses sharp clip corners (1px). Header is 9px tall but scales down for tiny rows.
    const radius = 1
    const headerH = Math.min(9, Math.max(7, Math.round(h * 0.4)))

    // Clip body — fuller saturation, less alpha bleed
    ctx.fillStyle = color
    ctx.globalAlpha = clip.muted ? 0.25 : 0.35
    ctx.beginPath()
    ctx.roundRect(x, y, w, h, radius)
    ctx.fill()
    ctx.globalAlpha = 1.0

    // Clip header bar — full-saturation color stripe
    ctx.fillStyle = clip.muted ? '#161620' : color
    ctx.globalAlpha = clip.muted ? 0.5 : 1.0
    ctx.beginPath()
    ctx.roundRect(x, y, w, headerH, [radius, radius, 0, 0])
    ctx.fill()
    ctx.globalAlpha = 1

    // Clip name — only render if there's enough room (header ≥ 8px)
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

    // Waveform
    const peaks = waveformData.get(clip.source_id)
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

    // Border (sharp 1px corner, with red glow when selected)
    if (isSelected) {
      ctx.strokeStyle = 'rgba(239,68,68,0.85)'
      ctx.lineWidth = 1
      ctx.shadowColor = 'rgba(239,68,68,0.55)'
      ctx.shadowBlur = 8
    } else {
      ctx.strokeStyle = clip.muted ? '#2a2a38' : 'rgba(0,0,0,0.5)'
      ctx.lineWidth = 1
      ctx.shadowBlur = 0
    }
    ctx.beginPath()
    ctx.roundRect(x, y, w, h, radius)
    ctx.stroke()
    ctx.shadowBlur = 0
  }

  // Hit test — accounts for current vertical scroll
  const hitTest = useCallback((mouseX: number, mouseY: number, scrollOffset: number): {
    clip: ClipInfo, trackId: string, edge: 'body' | 'right'
  } | null => {
    if (mouseY < RULER_HEIGHT) return null
    for (let i = 0; i < audioTracks.length; i++) {
      const y = RULER_HEIGHT + i * TRACK_HEIGHT - scrollYRef.current
      if (mouseY < y + 2 || mouseY > y + TRACK_HEIGHT - 2) continue
      const track = audioTracks[i]
      for (const clip of track.clips) {
        const clipX = clip.position_ticks * pixelsPerTick - scrollOffset
        const clipW = clip.length_ticks * pixelsPerTick
        if (mouseX >= clipX && mouseX <= clipX + clipW) {
          const edge = (mouseX > clipX + clipW - RESIZE_HANDLE_PX) ? 'right' : 'body'
          return { clip, trackId: track.id, edge }
        }
      }
    }
    return null
  }, [audioTracks, pixelsPerTick])

  const getScrollOffset = useCallback(() => {
    if (!autoFollowRef.current) return 0
    const playheadSecs = sampleRate > 0 ? positionSamples / sampleRate : 0
    const container = containerRef.current
    const w = container ? container.getBoundingClientRect().width : 800
    return Math.max(0, playheadSecs * ppsRef.current - w * 0.25)
  }, [positionSamples, sampleRate])

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    const rect = (e.target as HTMLElement).getBoundingClientRect()
    const mouseX = e.clientX - rect.left
    const mouseY = e.clientY - rect.top
    const scrollOffset = getScrollOffset()

    const hit = hitTest(mouseX, mouseY, scrollOffset)
    if (hit) {
      selectClip(hit.clip.id, hit.trackId)
      dragRef.current = {
        mode: hit.edge === 'right' ? 'resize-right' : 'move',
        clipId: hit.clip.id,
        trackId: hit.trackId,
        startMouseX: mouseX,
        originalPositionTicks: hit.clip.position_ticks,
        originalLengthTicks: hit.clip.length_ticks,
        altBypassedSnap: e.altKey,
      }
    } else {
      selectClip(null)
    }
  }, [hitTest, selectClip, getScrollOffset])

  const handleMouseMove = useCallback((e: React.MouseEvent) => {
    const drag = dragRef.current
    if (!drag || drag.mode === 'none') {
      const rect = (e.target as HTMLElement).getBoundingClientRect()
      const mouseX = e.clientX - rect.left
      const mouseY = e.clientY - rect.top
      const hit = hitTest(mouseX, mouseY, getScrollOffset())
      const canvas = canvasRef.current
      if (canvas) {
        canvas.style.cursor = hit ? (hit.edge === 'right' ? 'ew-resize' : 'grab') : 'default'
      }
      // Hint bar
      if (onSetHint) {
        if (hit) {
          onSetHint(hit.edge === 'right'
            ? `${hit.clip.name} · drag right edge to resize · hold Alt to bypass snap`
            : `${hit.clip.name} · drag to move · hold Alt to bypass snap`)
        } else if (mouseY < RULER_HEIGHT) {
          onSetHint('Timeline ruler · click to place playhead (TODO)')
        } else {
          onSetHint('Playlist · Ctrl+wheel = horizontal zoom · Alt+wheel = vertical zoom · wheel = scroll')
        }
      }
      return
    }

    const rect = (e.target as HTMLElement).getBoundingClientRect()
    const mouseX = e.clientX - rect.left
    const dx = mouseX - drag.startMouseX
    const dTicks = Math.round(dx / pixelsPerTick)
    // Snap-by-default. Hold Alt at any point during drag to bypass.
    const bypassSnap = e.altKey || drag.altBypassedSnap

    if (drag.mode === 'move') {
      const newPos = Math.max(0, drag.originalPositionTicks + dTicks)
      const finalPos = bypassSnap ? newPos : Math.round(newPos / PPQ) * PPQ
      moveClip(drag.trackId, drag.clipId, finalPos)
    } else if (drag.mode === 'resize-right') {
      const newLen = Math.max(PPQ, drag.originalLengthTicks + dTicks)
      const finalLen = bypassSnap ? newLen : Math.round(newLen / PPQ) * PPQ
      resizeClip(drag.trackId, drag.clipId, finalLen)
    }
  }, [hitTest, getScrollOffset, pixelsPerTick, moveClip, resizeClip, onSetHint])

  const handleMouseUp = useCallback(() => {
    dragRef.current = null
  }, [])

  const handleMouseLeave = useCallback(() => {
    dragRef.current = null
    if (onSetHint) onSetHint('')
  }, [onSetHint])

  // Two-axis zoom + scroll. Native event handler so we can preventDefault.
  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    const handler = (e: WheelEvent) => {
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault()
        const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1
        setPps(p => clamp(p * factor, PPS_MIN, PPS_MAX))
      } else if (e.altKey) {
        e.preventDefault()
        const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1
        setTrackH(h => clamp(Math.round(h * factor), TRACK_H_MIN, TRACK_H_MAX))
      } else {
        // Vertical pan — disengage auto-follow so the user keeps their position
        e.preventDefault()
        const totalH = audioTracks.length * trackHRef.current
        const visibleH = Math.max(1, container.clientHeight - RULER_HEIGHT)
        const maxScroll = Math.max(0, totalH - visibleH)
        setScrollY(y => clamp(y + e.deltaY, 0, maxScroll))
        if (autoFollowRef.current) setAutoFollow(false)
      }
    }
    container.addEventListener('wheel', handler, { passive: false })
    return () => container.removeEventListener('wheel', handler)
  }, [audioTracks.length])

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

      for (const file of files) {
        try {
          const result = await importAudioFile(trackId, file, offsetTicks)
          offsetTicks += result.length_ticks
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
        style={{ position: 'absolute', top: 0, left: 0 }}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
      />
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
