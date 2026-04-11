import { useRef, useEffect, useState, useCallback } from 'react'
import { listen } from '@tauri-apps/api/event'
import { useTrackStore, ClipInfo } from '../../stores/trackStore'
import { useTransportStore } from '../../stores/transportStore'
import { hw } from '../../theme'

const PPQ = 960
const TRACK_HEIGHT = 56
const PIXELS_PER_SECOND = 100
const RESIZE_HANDLE_PX = 6
const RULER_HEIGHT = 22

type DragMode = 'none' | 'move' | 'resize-right'

interface DragState {
  mode: DragMode
  clipId: string
  trackId: string
  startMouseX: number
  originalPositionTicks: number
  originalLengthTicks: number
}

const waveformData = new Map<string, [number, number][]>()

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

  const { tracks, selectedClipId, selectClip, moveClip, resizeClip, getWaveformPeaks } = useTrackStore()
  const { positionSamples, playing, bpm, sampleRate, setPosition } = useTransportStore()

  const audioTracks = tracks.filter(t => t.kind !== 'Master')
  const beatsPerSecond = bpm / 60
  const pixelsPerBeat = PIXELS_PER_SECOND / beatsPerSecond
  const pixelsPerTick = pixelsPerBeat / PPQ

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

    // Beat grid
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

    // Track lane backgrounds
    for (let i = 0; i < audioTracks.length; i++) {
      const y = RULER_HEIGHT + i * TRACK_HEIGHT
      ctx.fillStyle = i % 2 === 0 ? '#0a0a0f' : '#0c0c11'
      ctx.fillRect(0, y, w, TRACK_HEIGHT)

      ctx.strokeStyle = 'rgba(255,255,255,0.03)'
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(0, y + TRACK_HEIGHT)
      ctx.lineTo(w, y + TRACK_HEIGHT)
      ctx.stroke()
    }

    // Clips
    for (let i = 0; i < audioTracks.length; i++) {
      const y = RULER_HEIGHT + i * TRACK_HEIGHT
      const track = audioTracks[i]
      const clipColor = CLIP_COLORS[i % CLIP_COLORS.length]

      for (const clip of track.clips) {
        drawClip(ctx, clip, clipColor, y, scrollOffset, w, pixelsPerTick)
      }
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

  }, [tracks, positionSamples, playing, bpm, sampleRate, selectedClipId])

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
    const h = TRACK_HEIGHT - pad * 2
    const isSelected = clip.id === selectedClipId
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
    clip: ClipInfo, trackId: string, edge: 'body' | 'right'
  } | null => {
    for (let i = 0; i < audioTracks.length; i++) {
      const y = RULER_HEIGHT + i * TRACK_HEIGHT
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
    const playheadSecs = sampleRate > 0 ? positionSamples / sampleRate : 0
    const container = containerRef.current
    const w = container ? container.getBoundingClientRect().width : 800
    return Math.max(0, playheadSecs * PIXELS_PER_SECOND - w * 0.25)
  }, [positionSamples, sampleRate])

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    const rect = (e.target as HTMLElement).getBoundingClientRect()
    const mouseX = e.clientX - rect.left
    const mouseY = e.clientY - rect.top
    const scrollOffset = getScrollOffset()

    // Clicking the ruler strip seeks the transport to that position.
    if (mouseY < RULER_HEIGHT) {
      const seconds = Math.max(0, (mouseX + scrollOffset) / PIXELS_PER_SECOND)
      const samples = Math.round(seconds * (sampleRate || 48000))
      setPosition(samples)
      return
    }

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
        if (mouseY < RULER_HEIGHT) {
          canvas.style.cursor = 'pointer'
        } else {
          canvas.style.cursor = hit ? (hit.edge === 'right' ? 'ew-resize' : 'grab') : 'default'
        }
      }
      return
    }

    const rect = (e.target as HTMLElement).getBoundingClientRect()
    const mouseX = e.clientX - rect.left
    const dx = mouseX - drag.startMouseX
    const dTicks = Math.round(dx / pixelsPerTick)

    if (drag.mode === 'move') {
      const newPos = Math.max(0, drag.originalPositionTicks + dTicks)
      const snapped = Math.round(newPos / PPQ) * PPQ
      moveClip(drag.trackId, drag.clipId, snapped)
    } else if (drag.mode === 'resize-right') {
      const newLen = Math.max(PPQ, drag.originalLengthTicks + dTicks)
      const snapped = Math.round(newLen / PPQ) * PPQ
      resizeClip(drag.trackId, drag.clipId, snapped)
    }
  }, [hitTest, getScrollOffset, pixelsPerTick, moveClip, resizeClip])

  const handleMouseUp = useCallback(() => {
    dragRef.current = null
  }, [])

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
