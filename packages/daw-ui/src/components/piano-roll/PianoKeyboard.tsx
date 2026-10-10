import { useRef, useEffect, useCallback } from 'react'

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

      // A real keyboard: light white keys, dark black keys at 62% width,
      // so the octave reads at a glance. It was near-black on near-black.
      ctx.fillStyle = isC ? '#e4e4ea' : '#cfcfd6'
      ctx.fillRect(0, y, width, noteHeight)
      if (black) {
        ctx.fillStyle = '#17171c'
        ctx.fillRect(0, y + 0.5, width * 0.62, noteHeight - 1)
      }
      // Seam between white keys where two white keys meet (E-F, B-C).
      if (pitch % 12 === 4 || pitch % 12 === 11) {
        ctx.fillStyle = 'rgba(0,0,0,0.28)'
        ctx.fillRect(0, y, width, 1)
      } else if (!black) {
        ctx.fillStyle = 'rgba(0,0,0,0.12)'
        ctx.fillRect(width * 0.62, y + noteHeight - 0.5, width * 0.38, 0.5)
      }

      if (showScale && !inScale(pitch)) {
        ctx.fillStyle = 'rgba(0,0,0,0.45)'
        ctx.fillRect(0, y, width, noteHeight)
      }
      if (rootHere) {
        ctx.fillStyle = '#dc2626'
        ctx.fillRect(width - 3, y, 3, noteHeight)
      }

      // Octave label on every C; the other white keys get their letter
      // once the rows are tall enough to read it.
      if (isC || (noteHeight >= 14 && !black)) {
        ctx.fillStyle = isC ? '#18181b' : '#6b6b75'
        ctx.font = `${isC ? 600 : 500} ${Math.max(9, Math.min(11, noteHeight - 3))}px Inter, ui-sans-serif, sans-serif`
        ctx.textAlign = 'right'
        ctx.fillText(isC ? `C${Math.floor(pitch / 12) - 1}` : NOTE_NAMES[pitch % 12], width - 6, y + noteHeight - 3)
        ctx.textAlign = 'left'
      }
    }

    // Right border
    ctx.fillStyle = 'rgba(0,0,0,0.6)'
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
