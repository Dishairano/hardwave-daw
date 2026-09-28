import { useRef, useEffect, useCallback } from 'react'

interface Note {
  index: number
  startTick: number
  durationTicks: number
  pitch: number
  velocity: number
  muted: boolean
  pan: number
  fineCents: number
  releaseVelocity: number
}

/**
 * Which per-note property the lane edits.
 *
 * FL sets pan, fine pitch and release on a single note. The lane used to
 * do velocity only, so the other three had nowhere to live. They share
 * one strip and one dragging behaviour, because they are the same
 * gesture with a different number under it.
 */
export type NoteProperty = 'velocity' | 'pan' | 'fine' | 'release'

export const NOTE_PROPERTY_LABEL: Record<NoteProperty, string> = {
  velocity: 'VEL',
  pan: 'PAN',
  fine: 'FINE',
  release: 'REL',
}

/** Read the property off a note, normalised to 0..1 for the strip. */
function readNormalised(note: Note, property: NoteProperty): number {
  switch (property) {
    case 'velocity': return note.velocity
    case 'pan': return (note.pan + 1) / 2
    case 'fine': return (note.fineCents + 100) / 200
    case 'release': return note.releaseVelocity
  }
}

/** Turn a 0..1 strip position back into the property's own units. */
export function denormaliseNoteProperty(property: NoteProperty, t: number): number {
  switch (property) {
    case 'velocity': return Math.max(0.01, Math.min(1, t))
    case 'pan': return Math.max(-1, Math.min(1, t * 2 - 1))
    case 'fine': return Math.max(-100, Math.min(100, t * 200 - 100))
    case 'release': return Math.max(0, Math.min(1, t))
  }
}

/** Pan and fine pitch read from the middle, the others from the floor. */
function isCentred(property: NoteProperty): boolean {
  return property === 'pan' || property === 'fine'
}

interface VelocityLaneProps {
  notes: Note[]
  selectedNotes: Set<number>
  height: number
  keyboardWidth: number
  scrollX: number
  pixelsPerTick: number
  property: NoteProperty
  onVelocityChange: (index: number, value: number) => void
}

export function VelocityLane({
  notes, selectedNotes, height, keyboardWidth, scrollX, pixelsPerTick, property, onVelocityChange,
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

    // No label is drawn here: the buttons over the lane name the property
    // and drawing it twice put text under the first button.

    // A centre line, so a pan or a detune of zero is visible as zero.
    const centred = isCentred(property)
    if (centred) {
      ctx.strokeStyle = 'rgba(255,255,255,0.12)'
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(0, height / 2)
      ctx.lineTo(w, height / 2)
      ctx.stroke()
    }

    const barW = Math.max(3, 6 * pixelsPerTick)
    for (const note of notes) {
      const x = note.startTick * pixelsPerTick - scrollX
      if (x + barW < 0 || x > w) continue

      const t = readNormalised(note, property)
      const isSelected = selectedNotes.has(note.index)

      // A centred property grows out of the middle in both directions;
      // the others stand on the floor.
      const top = centred
        ? Math.min(height / 2, height * (1 - t))
        : height - t * (height - 4)
      const barH = centred
        ? Math.abs(height * (1 - t) - height / 2)
        : t * (height - 4)

      const gradient = ctx.createLinearGradient(0, top, 0, top + Math.max(barH, 1))
      if (t > 0.85) {
        gradient.addColorStop(0, '#EF4444')
        gradient.addColorStop(1, '#B91C1C')
      } else if (t > 0.5) {
        gradient.addColorStop(0, '#DC2626')
        gradient.addColorStop(1, '#991B1B')
      } else {
        gradient.addColorStop(0, '#991B1B')
        gradient.addColorStop(1, '#7F1D1D')
      }

      ctx.fillStyle = gradient
      ctx.globalAlpha = isSelected ? 1 : 0.7
      ctx.fillRect(x, top, barW, Math.max(barH, 1))
      ctx.globalAlpha = 1

      // Cap on the moving edge.
      ctx.fillStyle = isSelected ? '#fff' : 'rgba(255,255,255,0.4)'
      ctx.fillRect(x, top - 1, barW, 2)
    }
  }, [notes, selectedNotes, height, scrollX, pixelsPerTick, property])

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
    const newVel = denormaliseNoteProperty(property, 1 - my / height)

    if (draggedIdx != null) {
      onVelocityChange(draggedIdx, newVel)
      return
    }

    const idx = findNoteAtX(mx)
    if (idx != null) {
      draggingNoteRef.current = idx
      onVelocityChange(idx, newVel)
    }
  }, [findNoteAtX, height, onVelocityChange, property])

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
