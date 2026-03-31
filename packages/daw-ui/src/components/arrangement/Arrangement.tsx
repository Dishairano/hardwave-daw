import { useRef, useEffect, useState, useCallback } from 'react'
import { listen } from '@tauri-apps/api/event'
import { useTrackStore, ClipInfo } from '../../stores/trackStore'
import { useTransportStore } from '../../stores/transportStore'

const PPQ = 960
const TRACK_HEIGHT = 60
const PIXELS_PER_SECOND = 100
const RESIZE_HANDLE_PX = 6

type DragMode = 'none' | 'move' | 'resize-right'

interface DragState {
  mode: DragMode
  clipId: string
  trackId: string
  startMouseX: number
  originalPositionTicks: number
  originalLengthTicks: number
}

// Waveform cache
const waveformData = new Map<string, [number, number][]>()

export function Arrangement() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<DragState | null>(null)
  const [, forceRender] = useState(0)

  const { tracks, selectedClipId, selectClip, moveClip, resizeClip, getWaveformPeaks } = useTrackStore()
  const { positionSamples, playing, bpm, sampleRate } = useTransportStore()

  const audioTracks = tracks.filter(t => t.kind !== 'Master')
  const beatsPerSecond = bpm / 60
  const pixelsPerBeat = PIXELS_PER_SECOND / beatsPerSecond
  const pixelsPerTick = pixelsPerBeat / PPQ

  // Load waveforms for visible clips
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

    // Scroll
    const playheadSecs = sampleRate > 0 ? positionSamples / sampleRate : 0
    const scrollOffset = Math.max(0, playheadSecs * PIXELS_PER_SECOND - w * 0.25)

    // Background
    ctx.fillStyle = '#0a0a0b'
    ctx.fillRect(0, 0, w, h)

    // Beat grid
    ctx.lineWidth = 1
    const startBeat = Math.floor(scrollOffset / pixelsPerBeat)
    for (let i = startBeat; i < startBeat + Math.ceil(w / pixelsPerBeat) + 2; i++) {
      const x = i * pixelsPerBeat - scrollOffset
      if (x < -1 || x > w + 1) continue
      ctx.strokeStyle = i % 4 === 0 ? 'rgba(255,255,255,0.08)' : 'rgba(255,255,255,0.03)'
      ctx.beginPath()
      ctx.moveTo(x, 0)
      ctx.lineTo(x, h)
      ctx.stroke()
      if (i % 4 === 0 && i >= 0) {
        ctx.fillStyle = '#333'
        ctx.font = '9px monospace'
        ctx.fillText(`${Math.floor(i / 4) + 1}`, x + 3, 10)
      }
    }

    // Tracks and clips
    for (let i = 0; i < audioTracks.length; i++) {
      const y = 16 + i * TRACK_HEIGHT
      const track = audioTracks[i]

      // Track separator
      ctx.strokeStyle = 'rgba(255,255,255,0.04)'
      ctx.beginPath()
      ctx.moveTo(0, y + TRACK_HEIGHT)
      ctx.lineTo(w, y + TRACK_HEIGHT)
      ctx.stroke()

      ctx.fillStyle = 'rgba(255,255,255,0.01)'
      ctx.fillRect(0, y, w, TRACK_HEIGHT)

      // Clips
      for (const clip of track.clips) {
        drawClip(ctx, clip, track.color, track.id, y, scrollOffset, w, pixelsPerTick)
      }
    }

    // Playhead
    if (playing || positionSamples > 0) {
      const playheadX = playheadSecs * PIXELS_PER_SECOND - scrollOffset
      ctx.strokeStyle = '#ef4444'
      ctx.lineWidth = 1.5
      ctx.beginPath()
      ctx.moveTo(playheadX, 0)
      ctx.lineTo(playheadX, h)
      ctx.stroke()
      ctx.fillStyle = '#ef4444'
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
    trackColor: string,
    _trackId: string,
    trackY: number,
    scrollOffset: number,
    viewWidth: number,
    pxPerTick: number,
  ) {
    const clipX = clip.position_ticks * pxPerTick - scrollOffset
    const clipW = clip.length_ticks * pxPerTick
    if (clipX + clipW < 0 || clipX > viewWidth) return

    const y = trackY + 4
    const h = TRACK_HEIGHT - 8
    const isSelected = clip.id === selectedClipId
    const color = clip.muted ? '#333' : trackColor

    // Body
    ctx.fillStyle = clip.muted ? 'rgba(255,255,255,0.02)' : hexToRgba(color, 0.2)
    ctx.beginPath()
    ctx.roundRect(clipX, y, clipW, h, 4)
    ctx.fill()

    // Waveform
    const peaks = waveformData.get(clip.source_id)
    if (peaks && peaks.length > 0 && !clip.muted) {
      ctx.save()
      ctx.beginPath()
      ctx.roundRect(clipX + 1, y + 1, clipW - 2, h - 2, 3)
      ctx.clip()

      const midY = y + h * 0.5
      const ampScale = h * 0.4
      ctx.fillStyle = hexToRgba(color, 0.5)

      const pxPerBucket = clipW / peaks.length
      for (let j = 0; j < peaks.length; j++) {
        const bx = clipX + j * pxPerBucket
        if (bx + pxPerBucket < 0 || bx > viewWidth) continue
        const [minVal, maxVal] = peaks[j]
        const top = midY - maxVal * ampScale
        const bottom = midY - minVal * ampScale
        const barH = Math.max(1, bottom - top)
        ctx.fillRect(bx, top, Math.max(1, pxPerBucket - 0.5), barH)
      }

      ctx.restore()
    }

    // Border
    ctx.strokeStyle = isSelected
      ? '#fff'
      : clip.muted ? 'rgba(255,255,255,0.06)' : hexToRgba(color, 0.6)
    ctx.lineWidth = isSelected ? 1.5 : 1
    ctx.beginPath()
    ctx.roundRect(clipX, y, clipW, h, 4)
    ctx.stroke()

    // Name
    ctx.fillStyle = clip.muted ? '#444' : '#ddd'
    ctx.font = '10px system-ui, sans-serif'
    ctx.save()
    ctx.beginPath()
    ctx.rect(clipX, y, clipW, h)
    ctx.clip()
    ctx.fillText(clip.name, clipX + 6, y + 14)
    ctx.restore()
  }

  // Hit test: find which clip is under (mouseX, mouseY)
  const hitTest = useCallback((mouseX: number, mouseY: number, scrollOffset: number): {
    clip: ClipInfo, trackId: string, edge: 'body' | 'right'
  } | null => {
    for (let i = 0; i < audioTracks.length; i++) {
      const y = 16 + i * TRACK_HEIGHT
      if (mouseY < y + 4 || mouseY > y + TRACK_HEIGHT - 4) continue
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

  // Mouse handlers
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
      }
    } else {
      selectClip(null)
    }
  }, [hitTest, selectClip, getScrollOffset])

  const handleMouseMove = useCallback((e: React.MouseEvent) => {
    const drag = dragRef.current
    if (!drag || drag.mode === 'none') {
      // Update cursor
      const rect = (e.target as HTMLElement).getBoundingClientRect()
      const mouseX = e.clientX - rect.left
      const mouseY = e.clientY - rect.top
      const hit = hitTest(mouseX, mouseY, getScrollOffset())
      const canvas = canvasRef.current
      if (canvas) {
        canvas.style.cursor = hit ? (hit.edge === 'right' ? 'ew-resize' : 'grab') : 'default'
      }
      return
    }

    const rect = (e.target as HTMLElement).getBoundingClientRect()
    const mouseX = e.clientX - rect.left
    const dx = mouseX - drag.startMouseX
    const dTicks = Math.round(dx / pixelsPerTick)

    if (drag.mode === 'move') {
      const newPos = Math.max(0, drag.originalPositionTicks + dTicks)
      // Snap to beat
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

  // Drag-and-drop audio files from OS
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

      // Find or create a target track
      const state = useTrackStore.getState()
      let trackId = state.selectedTrackId
      const audio = state.tracks.filter(t => t.kind === 'Audio')
      if (!trackId || !audio.find(t => t.id === trackId)) {
        if (audio.length === 0) {
          await addAudioTrack()
          const updated = useTrackStore.getState()
          trackId = updated.tracks.find(t => t.kind === 'Audio')?.id || null
        } else {
          trackId = audio[0].id
        }
      }
      if (!trackId) return

      // Import each file sequentially, stacking them one after another
      let offsetTicks = 0
      // Find the end of existing clips on this track
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
        position: 'relative', overflow: 'hidden', background: '#0a0a0b',
        ...(dropHighlight ? { outline: '2px solid #7c3aed', outlineOffset: -2 } : {}),
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
          color: '#222', fontSize: 13, pointerEvents: 'none',
        }}>
          Add tracks to get started
        </div>
      )}
    </div>
  )
}

function hexToRgba(hex: string, alpha: number): string {
  const r = parseInt(hex.slice(1, 3), 16)
  const g = parseInt(hex.slice(3, 5), 16)
  const b = parseInt(hex.slice(5, 7), 16)
  return `rgba(${r},${g},${b},${alpha})`
}
