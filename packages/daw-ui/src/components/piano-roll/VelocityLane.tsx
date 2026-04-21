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
    ctx.fillStyle = '#08080d'
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
    ctx.fillStyle = 'rgba(255,255,255,0.04)'
    ctx.fillRect(0, 0, w, 1)

    // Label
    ctx.fillStyle = hw.textFaint
    ctx.font = '8px Inter, ui-sans-serif, sans-serif'
    ctx.fillText('VEL', 4, 12)

    // Velocity bars — red gradient
    const barW = Math.max(3, 6 * pixelsPerTick)
    for (const note of notes) {
      const x = note.startTick * pixelsPerTick - scrollX
      if (x + barW < 0 || x > w) continue

      const barH = note.velocity * (height - 4)
      const isSelected = selectedNotes.has(note.index)

      const gradient = ctx.createLinearGradient(0, height - barH, 0, height)
      if (note.velocity > 0.85) {
        gradient.addColorStop(0, '#EF4444')
        gradient.addColorStop(1, '#B91C1C')
      } else if (note.velocity > 0.5) {
        gradient.addColorStop(0, '#DC2626')
        gradient.addColorStop(1, '#991B1B')
      } else {
        gradient.addColorStop(0, '#991B1B')
        gradient.addColorStop(1, '#7F1D1D')
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

  const draggingNoteRef = useRef<number | null>(null)
  const curveModeRef = useRef(false)
  const touchedIndicesRef = useRef<Set<number>>(new Set())

  const findNoteAtX = useCallback((mx: number): number | null => {
    const barW = Math.max(3, 6 * pixelsPerTick)
    for (const note of notes) {
      const x = note.startTick * pixelsPerTick - scrollX
      if (mx >= x && mx <= x + barW) return note.index
    }
    return null
  }, [notes, scrollX, pixelsPerTick])

  const applyVelAt = useCallback((clientX: number, clientY: number, draggedIdx?: number) => {
    const rect = canvasRef.current!.getBoundingClientRect()
    const my = clientY - rect.top
    const mx = clientX - rect.left
    const newVel = Math.max(0.01, Math.min(1, 1 - my / height))

    if (draggedIdx != null) {
      onVelocityChange(draggedIdx, newVel)
      return
    }

    const idx = findNoteAtX(mx)
    if (idx != null) {
      draggingNoteRef.current = idx
      onVelocityChange(idx, newVel)
    }
  }, [findNoteAtX, height, onVelocityChange])

  const handleMouseDown = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    draggingNoteRef.current = null
    curveModeRef.current = e.shiftKey
    touchedIndicesRef.current = new Set()
    applyVelAt(e.clientX, e.clientY)

    const onMove = (ev: MouseEvent) => {
      if (curveModeRef.current) {
        const rect = canvasRef.current!.getBoundingClientRect()
        const mx = ev.clientX - rect.left
        const idx = findNoteAtX(mx)
        if (idx != null && !touchedIndicesRef.current.has(idx)) {
          touchedIndicesRef.current.add(idx)
          applyVelAt(ev.clientX, ev.clientY, idx)
        } else if (idx != null) {
          applyVelAt(ev.clientX, ev.clientY, idx)
        }
        return
      }
      const idx = draggingNoteRef.current
      if (idx == null) return
      applyVelAt(ev.clientX, ev.clientY, idx)
    }
    const onUp = () => {
      draggingNoteRef.current = null
      curveModeRef.current = false
      touchedIndicesRef.current = new Set()
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }, [applyVelAt, findNoteAtX])

  return (
    <div ref={containerRef} data-testid="velocity-lane" style={{
      height, marginLeft: keyboardWidth,
      borderTop: `1px solid rgba(255,255,255,0.04)`,
      overflow: 'hidden',
    }}>
      <canvas
        ref={canvasRef}
        onMouseDown={handleMouseDown}
        title="Click/drag to set velocity, Shift+drag across notes to draw a curve"
        style={{ display: 'block', cursor: 'ns-resize' }}
      />
    </div>
  )
}
