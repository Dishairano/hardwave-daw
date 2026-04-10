import { useRef, useEffect, useCallback } from 'react'
import { hw } from '../../theme'

interface Note {
  index: number
  startTick: number
  durationTicks: number
  pitch: number
  velocity: number
  muted: boolean
}

interface VelocityLaneProps {
  notes: Note[]
  selectedNotes: Set<number>
  height: number
  keyboardWidth: number
  scrollX: number
  pixelsPerTick: number
  onVelocityChange: (index: number, velocity: number) => void
}

export function VelocityLane({
  notes, selectedNotes, height, keyboardWidth, scrollX, pixelsPerTick, onVelocityChange,
}: VelocityLaneProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  const draw = useCallback(() => {
    const canvas = canvasRef.current
    const container = containerRef.current
    if (!canvas || !container) return

    const w = container.clientWidth
    canvas.width = w * devicePixelRatio
    canvas.height = height * devicePixelRatio
    canvas.style.width = `${w}px`
    canvas.style.height = `${height}px`

    const ctx = canvas.getContext('2d')!
    ctx.scale(devicePixelRatio, devicePixelRatio)

    // Background
    ctx.fillStyle = hw.bgInput
    ctx.fillRect(0, 0, w, height)

    // Guide lines
    ctx.strokeStyle = 'rgba(255, 255, 255, 0.03)'
    ctx.lineWidth = 0.5
    for (const pct of [0.25, 0.5, 0.75]) {
      const y = height * (1 - pct)
      ctx.beginPath()
      ctx.moveTo(0, y)
      ctx.lineTo(w, y)
      ctx.stroke()
    }

    // Top border
    ctx.fillStyle = hw.borderDark
    ctx.fillRect(0, 0, w, 1)

    // Label
    ctx.fillStyle = hw.textFaint
    ctx.font = '8px Segoe UI, sans-serif'
    ctx.fillText('VEL', 4, 12)

    // Velocity bars — purple gradient
    const barW = Math.max(3, 6 * pixelsPerTick)
    for (const note of notes) {
      const x = note.startTick * pixelsPerTick - scrollX
      if (x + barW < 0 || x > w) continue

      const barH = note.velocity * (height - 4)
      const isSelected = selectedNotes.has(note.index)

      const gradient = ctx.createLinearGradient(0, height - barH, 0, height)
      if (note.velocity > 0.85) {
        gradient.addColorStop(0, '#FF4466')
        gradient.addColorStop(1, '#CC2244')
      } else if (note.velocity > 0.5) {
        gradient.addColorStop(0, '#9B6DFF')
        gradient.addColorStop(1, '#7B5AC0')
      } else {
        gradient.addColorStop(0, '#7B5AC0')
        gradient.addColorStop(1, '#5A3FA0')
      }

      ctx.fillStyle = gradient
      ctx.globalAlpha = isSelected ? 1 : 0.7
      ctx.fillRect(x, height - barH, barW, barH)
      ctx.globalAlpha = 1

      // Top cap
      ctx.fillStyle = isSelected ? '#fff' : 'rgba(255,255,255,0.4)'
      ctx.fillRect(x, height - barH - 1, barW, 2)
    }
  }, [notes, selectedNotes, height, scrollX, pixelsPerTick])

  useEffect(() => { draw() }, [draw])

  useEffect(() => {
    const obs = new ResizeObserver(() => draw())
    if (containerRef.current) obs.observe(containerRef.current)
    return () => obs.disconnect()
  }, [draw])

  const handleMouseDown = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = canvasRef.current!.getBoundingClientRect()
    const my = e.clientY - rect.top
    const mx = e.clientX - rect.left

    const barW = Math.max(3, 6 * pixelsPerTick)

    for (const note of notes) {
      const x = note.startTick * pixelsPerTick - scrollX
      if (mx >= x && mx <= x + barW) {
        const newVel = Math.max(0.01, Math.min(1, 1 - my / height))
        onVelocityChange(note.index, newVel)
        break
      }
    }
  }, [notes, scrollX, pixelsPerTick, height, onVelocityChange])

  return (
    <div ref={containerRef} data-testid="velocity-lane" style={{
      height, marginLeft: keyboardWidth,
      borderTop: `1px solid ${hw.borderDark}`,
      overflow: 'hidden',
    }}>
      <canvas
        ref={canvasRef}
        onMouseDown={handleMouseDown}
        style={{ display: 'block', cursor: 'ns-resize' }}
      />
    </div>
  )
}
