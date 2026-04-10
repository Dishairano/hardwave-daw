import { useRef, useEffect, useCallback } from 'react'
import { hw } from '../../theme'

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']

interface PianoKeyboardProps {
  width: number
  noteHeight: number
  scrollY: number
  totalNotes: number
}

export function PianoKeyboard({ width, noteHeight, scrollY, totalNotes }: PianoKeyboardProps) {
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
    ctx.fillStyle = hw.bgPanel
    ctx.fillRect(0, 0, width, h)

    for (let pitch = 0; pitch < totalNotes; pitch++) {
      const y = (totalNotes - 1 - pitch) * noteHeight - scrollY
      if (y + noteHeight < 0 || y > h) continue

      const black = isBlack(pitch)
      const isC = pitch % 12 === 0

      if (black) {
        ctx.fillStyle = '#0a0a10'
        ctx.fillRect(0, y, width, noteHeight)
        ctx.fillStyle = '#080810'
        ctx.fillRect(0, y + 1, width * 0.6, noteHeight - 2)
      } else {
        ctx.fillStyle = isC ? '#17171e' : '#111118'
        ctx.fillRect(0, y, width, noteHeight)
      }

      // Key border
      ctx.fillStyle = pitch % 12 === 0 || pitch % 12 === 5
        ? 'rgba(155, 109, 255, 0.06)'
        : 'rgba(255,255,255,0.02)'
      ctx.fillRect(0, y + noteHeight - 0.5, width, 0.5)

      // Note label
      if (isC || (noteHeight >= 14 && !black)) {
        const label = isC
          ? `C${Math.floor(pitch / 12) - 1}`
          : NOTE_NAMES[pitch % 12]
        ctx.fillStyle = isC ? hw.textSecondary : hw.textFaint
        ctx.font = isC
          ? `bold ${Math.min(10, noteHeight - 3)}px Segoe UI, sans-serif`
          : `${Math.min(8, noteHeight - 4)}px Segoe UI, sans-serif`
        ctx.fillText(label, 4, y + noteHeight - 3)
      }
    }

    // Right border
    ctx.fillStyle = hw.borderDark
    ctx.fillRect(width - 1, 0, 1, h)
  }, [width, noteHeight, scrollY, totalNotes])

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
