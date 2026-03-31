import { useRef, useEffect } from 'react'
import { useTrackStore } from '../../stores/trackStore'
import { useTransportStore } from '../../stores/transportStore'

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

    // Background
    ctx.fillStyle = '#0a0a0b'
    ctx.fillRect(0, 0, w, h)

    // Grid lines (beats)
    const beatsPerSecond = bpm / 60
    const pixelsPerBeat = pixelsPerSecond / beatsPerSecond
    const scrollOffset = (positionSamples / sampleRate) * pixelsPerSecond

    ctx.strokeStyle = 'rgba(255,255,255,0.04)'
    ctx.lineWidth = 1
    const startBeat = Math.floor(scrollOffset / pixelsPerBeat)
    for (let i = startBeat; i < startBeat + Math.ceil(w / pixelsPerBeat) + 1; i++) {
      const x = i * pixelsPerBeat - scrollOffset
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

    // Track lanes
    const audioTracks = tracks.filter(t => t.kind !== 'Master')
    for (let i = 0; i < audioTracks.length; i++) {
      const y = 16 + i * trackHeight

      // Track separator
      ctx.strokeStyle = 'rgba(255,255,255,0.04)'
      ctx.beginPath()
      ctx.moveTo(0, y + trackHeight)
      ctx.lineTo(w, y + trackHeight)
      ctx.stroke()

      // Track background (slightly different for selected)
      ctx.fillStyle = 'rgba(255,255,255,0.01)'
      ctx.fillRect(0, y, w, trackHeight)
    }

    // Playhead
    if (playing || positionSamples > 0) {
      const playheadX = (positionSamples / sampleRate) * pixelsPerSecond - scrollOffset
      ctx.strokeStyle = '#ef4444'
      ctx.lineWidth = 1.5
      ctx.beginPath()
      ctx.moveTo(playheadX, 0)
      ctx.lineTo(playheadX, h)
      ctx.stroke()
    }

  }, [tracks, positionSamples, playing, bpm, sampleRate])

  return (
    <div ref={containerRef} style={{ position: 'relative', overflow: 'hidden', background: '#0a0a0b' }}>
      <canvas ref={canvasRef} style={{ position: 'absolute', top: 0, left: 0 }} />
      {tracks.length === 0 && (
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
