import { useRef, useEffect } from 'react'
import { useTrackStore } from '../../stores/trackStore'
import { useTransportStore } from '../../stores/transportStore'

const PPQ = 960

export function Arrangement() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const { tracks } = useTrackStore()
  const { positionSamples, playing, bpm, sampleRate } = useTransportStore()

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
    const trackHeight = 60
    const pixelsPerSecond = 100
    const beatsPerSecond = bpm / 60
    const pixelsPerBeat = pixelsPerSecond / beatsPerSecond
    const pixelsPerTick = pixelsPerBeat / PPQ

    // Scroll so playhead stays ~1/4 from left when playing
    const playheadSecs = sampleRate > 0 ? positionSamples / sampleRate : 0
    const scrollOffset = playing
      ? Math.max(0, playheadSecs * pixelsPerSecond - w * 0.25)
      : Math.max(0, playheadSecs * pixelsPerSecond - w * 0.25)

    // Background
    ctx.fillStyle = '#0a0a0b'
    ctx.fillRect(0, 0, w, h)

    // Grid lines (beats)
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

      // Bar numbers
      if (i % 4 === 0 && i >= 0) {
        ctx.fillStyle = '#333'
        ctx.font = '9px monospace'
        ctx.fillText(`${Math.floor(i / 4) + 1}`, x + 3, 10)
      }
    }

    // Track lanes and clips
    const audioTracks = tracks.filter(t => t.kind !== 'Master')
    for (let i = 0; i < audioTracks.length; i++) {
      const y = 16 + i * trackHeight
      const track = audioTracks[i]

      // Track separator
      ctx.strokeStyle = 'rgba(255,255,255,0.04)'
      ctx.beginPath()
      ctx.moveTo(0, y + trackHeight)
      ctx.lineTo(w, y + trackHeight)
      ctx.stroke()

      // Track background
      ctx.fillStyle = 'rgba(255,255,255,0.01)'
      ctx.fillRect(0, y, w, trackHeight)

      // Render clips
      if (track.clips) {
        for (const clip of track.clips) {
          const clipStartPx = clip.position_ticks * pixelsPerTick - scrollOffset
          const clipWidthPx = clip.length_ticks * pixelsPerTick

          // Skip if not visible
          if (clipStartPx + clipWidthPx < 0 || clipStartPx > w) continue

          const clipY = y + 4
          const clipH = trackHeight - 8

          // Clip body
          const color = track.color || '#7c3aed'
          ctx.fillStyle = clip.muted ? 'rgba(255,255,255,0.03)' : hexToRgba(color, 0.3)
          ctx.beginPath()
          ctx.roundRect(clipStartPx, clipY, clipWidthPx, clipH, 4)
          ctx.fill()

          // Clip border
          ctx.strokeStyle = clip.muted ? 'rgba(255,255,255,0.06)' : hexToRgba(color, 0.6)
          ctx.lineWidth = 1
          ctx.beginPath()
          ctx.roundRect(clipStartPx, clipY, clipWidthPx, clipH, 4)
          ctx.stroke()

          // Clip name
          ctx.fillStyle = clip.muted ? '#444' : '#ddd'
          ctx.font = '10px system-ui, sans-serif'
          ctx.save()
          ctx.beginPath()
          ctx.rect(clipStartPx, clipY, clipWidthPx, clipH)
          ctx.clip()
          ctx.fillText(clip.name, clipStartPx + 6, clipY + 14)
          ctx.restore()

          // Waveform placeholder lines
          if (!clip.muted && clipWidthPx > 20) {
            ctx.strokeStyle = hexToRgba(color, 0.4)
            ctx.lineWidth = 0.5
            const waveY = clipY + clipH * 0.5
            const step = Math.max(2, Math.floor(clipWidthPx / 80))
            ctx.beginPath()
            for (let px = clipStartPx + 4; px < clipStartPx + clipWidthPx - 4; px += step) {
              const amp = (Math.sin(px * 0.3) * 0.3 + Math.sin(px * 0.7) * 0.2) * (clipH * 0.3)
              ctx.moveTo(px, waveY - amp)
              ctx.lineTo(px, waveY + amp)
            }
            ctx.stroke()
          }
        }
      }
    }

    // Playhead
    if (playing || positionSamples > 0) {
      const playheadX = playheadSecs * pixelsPerSecond - scrollOffset
      ctx.strokeStyle = '#ef4444'
      ctx.lineWidth = 1.5
      ctx.beginPath()
      ctx.moveTo(playheadX, 0)
      ctx.lineTo(playheadX, h)
      ctx.stroke()

      // Playhead triangle
      ctx.fillStyle = '#ef4444'
      ctx.beginPath()
      ctx.moveTo(playheadX - 5, 0)
      ctx.lineTo(playheadX + 5, 0)
      ctx.lineTo(playheadX, 7)
      ctx.closePath()
      ctx.fill()
    }

  }, [tracks, positionSamples, playing, bpm, sampleRate])

  return (
    <div ref={containerRef} style={{ position: 'relative', overflow: 'hidden', background: '#0a0a0b' }}>
      <canvas ref={canvasRef} style={{ position: 'absolute', top: 0, left: 0 }} />
      {tracks.filter(t => t.kind !== 'Master').length === 0 && (
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
