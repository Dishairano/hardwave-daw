import { useRef, useEffect, useCallback } from 'react'
import { hw } from '../../theme'

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']

interface PianoKeyboardProps {
  width: number
  noteHeight: number
  scrollY: number
  totalNotes: number
  scaleRoot?: number
  scaleIntervals?: number[]
  showScale?: boolean
}

export function PianoKeyboard({
  width, noteHeight, scrollY, totalNotes,
  scaleRoot = 0, scaleIntervals, showScale = false,
}: PianoKeyboardProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  const isBlack = (pitch: number) => [1, 3, 6, 8, 10].includes(pitch % 12)

  const draw = useCallback(() => {
    const canvas = canvasRef.current
    const container = containerRef.current
    if (!canvas || !container) return

    const h = container.clientHeight
    canvas.width = width * devicePixelRatio
    canvas.height = h * devicePixelRatio
    canvas.style.width = `${width}px`
    canvas.style.height = `${h}px`

    const ctx = canvas.getContext('2d')!
    ctx.scale(devicePixelRatio, devicePixelRatio)

    // Background
    ctx.fillStyle = '#0a0a0f'
    ctx.fillRect(0, 0, width, h)

    const inScale = (pitch: number) => {
      if (!showScale || !scaleIntervals) return true
      const rel = ((pitch - scaleRoot) % 12 + 12) % 12
      return scaleIntervals.includes(rel)
    }

    for (let pitch = 0; pitch < totalNotes; pitch++) {
      const y = (totalNotes - 1 - pitch) * noteHeight - scrollY
      if (y + noteHeight < 0 || y > h) continue

      const black = isBlack(pitch)
      const isC = pitch % 12 === 0
      const rootHere = showScale && ((pitch - scaleRoot) % 12 + 12) % 12 === 0

      if (black) {
        ctx.fillStyle = '#08080d'
        ctx.fillRect(0, y, width, noteHeight)
        ctx.fillStyle = '#060608'
        ctx.fillRect(0, y + 1, width * 0.6, noteHeight - 2)
      } else {
        ctx.fillStyle = isC ? '#0d0d12' : '#0a0a0f'
        ctx.fillRect(0, y, width, noteHeight)
      }

      if (showScale && !inScale(pitch)) {
        ctx.fillStyle = 'rgba(0,0,0,0.45)'
        ctx.fillRect(0, y, width, noteHeight)
      }
      if (rootHere) {
        ctx.fillStyle = 'rgba(220,38,38,0.18)'
        ctx.fillRect(0, y, 3, noteHeight)
      }

      // Key border
      ctx.fillStyle = pitch % 12 === 0 || pitch % 12 === 5
        ? 'rgba(220,38,38,0.06)'
        : 'rgba(255,255,255,0.02)'
      ctx.fillRect(0, y + noteHeight - 0.5, width, 0.5)

      // Note label
      if (isC || (noteHeight >= 14 && !black)) {
        const label = isC
          ? `C${Math.floor(pitch / 12) - 1}`
          : NOTE_NAMES[pitch % 12]
        ctx.fillStyle = isC ? hw.textSecondary : hw.textFaint
        ctx.font = isC
          ? `bold ${Math.min(10, noteHeight - 3)}px Inter, ui-sans-serif, sans-serif`
          : `${Math.min(8, noteHeight - 4)}px Inter, ui-sans-serif, sans-serif`
        ctx.fillText(label, 4, y + noteHeight - 3)
      }
    }

    // Right border
    ctx.fillStyle = 'rgba(255,255,255,0.04)'
    ctx.fillRect(width - 1, 0, 1, h)
  }, [width, noteHeight, scrollY, totalNotes, scaleRoot, scaleIntervals, showScale])

  useEffect(() => { draw() }, [draw])

  useEffect(() => {
    const obs = new ResizeObserver(() => draw())
    if (containerRef.current) obs.observe(containerRef.current)
    return () => obs.disconnect()
  }, [draw])

  return (
    <div ref={containerRef} style={{ width, flexShrink: 0, overflow: 'hidden' }}>
      <canvas ref={canvasRef} />
    </div>
  )
}
