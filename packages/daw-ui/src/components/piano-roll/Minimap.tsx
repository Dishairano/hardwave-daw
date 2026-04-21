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

interface MinimapProps {
  notes: Note[]
  height: number
  keyboardWidth: number
  scrollX: number
  viewWidth: number
  totalTicks: number
  pixelsPerTick: number
  onScroll: (scrollX: number) => void
}

export function Minimap({
  notes, height, keyboardWidth, scrollX, viewWidth, totalTicks, pixelsPerTick, onScroll,
}: MinimapProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const draggingRef = useRef(false)

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

    ctx.fillStyle = '#08080d'
    ctx.fillRect(0, 0, w, height)

    const label = 'MAP'
    ctx.fillStyle = hw.textFaint
    ctx.font = '8px Inter, ui-sans-serif, sans-serif'
    ctx.fillText(label, 4, 10)

    if (totalTicks <= 0 || notes.length === 0) return

    const pitches = notes.map(n => n.pitch)
    const minPitch = Math.max(0, Math.min(...pitches) - 2)
    const maxPitch = Math.min(127, Math.max(...pitches) + 2)
    const pitchRange = Math.max(1, maxPitch - minPitch)

    for (const note of notes) {
      const x = (note.startTick / totalTicks) * w
      const nw = Math.max(1, (note.durationTicks / totalTicks) * w)
      const y = ((maxPitch - note.pitch) / pitchRange) * (height - 4) + 2
      ctx.fillStyle = '#DC2626'
      ctx.globalAlpha = 0.6
      ctx.fillRect(x, y, nw, 1.5)
      ctx.globalAlpha = 1
    }

    const viewTicks = viewWidth / pixelsPerTick
    const viewportX = (scrollX / pixelsPerTick / totalTicks) * w
    const viewportW = Math.min(w - viewportX, (viewTicks / totalTicks) * w)
    ctx.fillStyle = 'rgba(220,38,38,0.12)'
    ctx.fillRect(viewportX, 0, viewportW, height)
    ctx.strokeStyle = 'rgba(220,38,38,0.55)'
    ctx.lineWidth = 1
    ctx.strokeRect(viewportX + 0.5, 0.5, Math.max(1, viewportW - 1), height - 1)
  }, [notes, height, scrollX, viewWidth, totalTicks, pixelsPerTick])

  useEffect(() => { draw() }, [draw])

  useEffect(() => {
    const obs = new ResizeObserver(() => draw())
    if (containerRef.current) obs.observe(containerRef.current)
    return () => obs.disconnect()
  }, [draw])

  const applyFromClientX = useCallback((clientX: number) => {
    const container = containerRef.current
    if (!container) return
    const rect = container.getBoundingClientRect()
    const mx = clientX - rect.left
    const w = container.clientWidth
    if (w <= 0 || totalTicks <= 0) return
    const viewTicks = viewWidth / pixelsPerTick
    const tickAtCenter = (mx / w) * totalTicks
    const targetTick = Math.max(0, tickAtCenter - viewTicks / 2)
    onScroll(targetTick * pixelsPerTick)
  }, [viewWidth, totalTicks, pixelsPerTick, onScroll])

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    draggingRef.current = true
    applyFromClientX(e.clientX)
    const onMove = (ev: MouseEvent) => {
      if (draggingRef.current) applyFromClientX(ev.clientX)
    }
    const onUp = () => {
      draggingRef.current = false
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }, [applyFromClientX])

  return (
    <div ref={containerRef} data-testid="piano-minimap" style={{
      height, marginLeft: keyboardWidth,
      borderTop: `1px solid rgba(255,255,255,0.04)`,
      overflow: 'hidden',
    }}>
      <canvas
        ref={canvasRef}
        onMouseDown={handleMouseDown}
        title="Drag to jump to region"
        style={{ display: 'block', cursor: 'ew-resize' }}
      />
    </div>
  )
}
