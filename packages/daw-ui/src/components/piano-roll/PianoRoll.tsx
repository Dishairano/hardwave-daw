import { useEffect, useRef, useState, useCallback } from 'react'
import { hw } from '../../theme'
import { PianoKeyboard } from './PianoKeyboard'
import { VelocityLane } from './VelocityLane'

const PPQ = 960
const NOTE_HEIGHT = 14
const RULER_HEIGHT = 22
const KEYBOARD_WIDTH = 60
const VELOCITY_LANE_HEIGHT = 80
const TOTAL_NOTES = 128 // C-1 to G9
const DEFAULT_SNAP = PPQ / 4 // 16th note

interface Note {
  index: number
  startTick: number
  durationTicks: number
  pitch: number
  velocity: number
  muted: boolean
  selected?: boolean
}

interface PianoRollProps {
  trackId?: string
  clipId?: string
}

export function PianoRoll({ trackId, clipId }: PianoRollProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const [notes, setNotes] = useState<Note[]>([])
  const [scrollX, setScrollX] = useState(0)
  const [scrollY, setScrollY] = useState(NOTE_HEIGHT * 60) // Start around C4
  const [pixelsPerTick, setPixelsPerTick] = useState(0.12)
  const [snap, setSnap] = useState(DEFAULT_SNAP)
  const [selectedNotes, setSelectedNotes] = useState<Set<number>>(new Set())
  const [tool, setTool] = useState<'draw' | 'select' | 'erase'>('draw')
  const dragRef = useRef<{
    mode: 'none' | 'draw' | 'move' | 'resize' | 'select-box' | 'velocity'
    noteIndex: number
    startX: number
    startY: number
    origTick: number
    origPitch: number
    origDuration: number
  }>({ mode: 'none', noteIndex: -1, startX: 0, startY: 0, origTick: 0, origPitch: 0, origDuration: 0 })

  const totalWidth = PPQ * 4 * 32 * pixelsPerTick // 32 bars
  const totalHeight = TOTAL_NOTES * NOTE_HEIGHT

  const snapTick = (tick: number) => Math.round(tick / snap) * snap
  const pitchFromY = (y: number) => TOTAL_NOTES - 1 - Math.floor((y + scrollY) / NOTE_HEIGHT)
  const tickFromX = (x: number) => (x - KEYBOARD_WIDTH + scrollX) / pixelsPerTick
  const yFromPitch = (pitch: number) => (TOTAL_NOTES - 1 - pitch) * NOTE_HEIGHT - scrollY
  const xFromTick = (tick: number) => tick * pixelsPerTick - scrollX + KEYBOARD_WIDTH

  // Note name helper
  const noteName = (pitch: number) => {
    const names = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
    return `${names[pitch % 12]}${Math.floor(pitch / 12) - 1}`
  }

  const isBlackKey = (pitch: number) => [1, 3, 6, 8, 10].includes(pitch % 12)

  // Draw the grid canvas
  const draw = useCallback(() => {
    const canvas = canvasRef.current
    const container = containerRef.current
    if (!canvas || !container) return

    const rect = container.getBoundingClientRect()
    const w = rect.width - KEYBOARD_WIDTH
    const h = rect.height - RULER_HEIGHT - VELOCITY_LANE_HEIGHT
    canvas.width = w * devicePixelRatio
    canvas.height = h * devicePixelRatio
    canvas.style.width = `${w}px`
    canvas.style.height = `${h}px`

    const ctx = canvas.getContext('2d')!
    ctx.scale(devicePixelRatio, devicePixelRatio)

    // Background
    ctx.fillStyle = '#363636'
    ctx.fillRect(0, 0, w, h)

    // Draw pitch rows (alternating shade for black keys)
    for (let pitch = 0; pitch < TOTAL_NOTES; pitch++) {
      const y = yFromPitch(pitch)
      if (y + NOTE_HEIGHT < 0 || y > h) continue
      if (isBlackKey(pitch)) {
        ctx.fillStyle = '#303030'
        ctx.fillRect(0, y, w, NOTE_HEIGHT)
      }
      // C note highlight
      if (pitch % 12 === 0) {
        ctx.fillStyle = 'rgba(255, 255, 255, 0.03)'
        ctx.fillRect(0, y, w, NOTE_HEIGHT)
      }
    }

    // Horizontal grid lines (per note)
    ctx.strokeStyle = 'rgba(255, 255, 255, 0.03)'
    ctx.lineWidth = 0.5
    for (let pitch = 0; pitch < TOTAL_NOTES; pitch++) {
      const y = yFromPitch(pitch) + NOTE_HEIGHT
      if (y < 0 || y > h) continue
      ctx.beginPath()
      ctx.moveTo(0, y)
      ctx.lineTo(w, y)
      ctx.stroke()
    }

    // Vertical grid lines (beats and bars)
    const startTick = Math.max(0, Math.floor(scrollX / pixelsPerTick / PPQ) * PPQ)
    const endTick = (scrollX + w) / pixelsPerTick

    for (let tick = startTick; tick <= endTick; tick += PPQ / 4) {
      const x = xFromTick(tick) - KEYBOARD_WIDTH
      if (x < 0 || x > w) continue

      const isBar = tick % (PPQ * 4) === 0
      const isBeat = tick % PPQ === 0

      if (isBar) {
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.12)'
        ctx.lineWidth = 1
      } else if (isBeat) {
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.06)'
        ctx.lineWidth = 0.5
      } else {
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.025)'
        ctx.lineWidth = 0.5
      }

      ctx.beginPath()
      ctx.moveTo(x, 0)
      ctx.lineTo(x, h)
      ctx.stroke()
    }

    // Draw notes
    for (const note of notes) {
      const x = xFromTick(note.startTick) - KEYBOARD_WIDTH
      const y = yFromPitch(note.pitch)
      const noteW = note.durationTicks * pixelsPerTick
      if (x + noteW < 0 || x > w || y + NOTE_HEIGHT < 0 || y > h) continue

      const isSelected = selectedNotes.has(note.index)
      const color = note.muted ? '#666' : '#00CC44'

      // Note body
      ctx.fillStyle = isSelected ? '#44FF66' : color
      ctx.globalAlpha = note.muted ? 0.4 : 0.85
      ctx.beginPath()
      ctx.roundRect(x + 0.5, y + 1, Math.max(noteW - 1, 2), NOTE_HEIGHT - 2, 2)
      ctx.fill()
      ctx.globalAlpha = 1

      // Note border
      ctx.strokeStyle = isSelected ? '#fff' : 'rgba(255,255,255,0.15)'
      ctx.lineWidth = isSelected ? 1.5 : 0.5
      ctx.beginPath()
      ctx.roundRect(x + 0.5, y + 1, Math.max(noteW - 1, 2), NOTE_HEIGHT - 2, 2)
      ctx.stroke()

      // Note name label (if wide enough)
      if (noteW > 30) {
        ctx.fillStyle = isSelected ? '#fff' : 'rgba(255,255,255,0.7)'
        ctx.font = '9px Segoe UI, sans-serif'
        ctx.fillText(noteName(note.pitch), x + 4, y + NOTE_HEIGHT - 3)
      }

      // Velocity brightness indicator (left edge bar)
      const velH = (NOTE_HEIGHT - 4) * note.velocity
      ctx.fillStyle = `rgba(255, 255, 255, ${0.1 + note.velocity * 0.2})`
      ctx.fillRect(x + 1, y + NOTE_HEIGHT - 1 - velH, 2, velH)

      // Resize handle (right edge)
      ctx.fillStyle = 'rgba(255, 255, 255, 0.25)'
      ctx.fillRect(x + noteW - 4, y + 3, 2, NOTE_HEIGHT - 6)
    }
  }, [notes, scrollX, scrollY, pixelsPerTick, selectedNotes])

  useEffect(() => {
    draw()
  }, [draw])

  // Re-draw on resize
  useEffect(() => {
    const obs = new ResizeObserver(() => draw())
    if (containerRef.current) obs.observe(containerRef.current)
    return () => obs.disconnect()
  }, [draw])

  // Mouse handlers on canvas
  const handleMouseDown = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = canvasRef.current!.getBoundingClientRect()
    const mx = e.clientX - rect.left
    const my = e.clientY - rect.top
    const tick = tickFromX(mx + KEYBOARD_WIDTH)
    const pitch = pitchFromY(my + RULER_HEIGHT)

    // Check if clicking on existing note
    for (const note of notes) {
      const nx = xFromTick(note.startTick) - KEYBOARD_WIDTH
      const ny = yFromPitch(note.pitch)
      const nw = note.durationTicks * pixelsPerTick

      if (mx >= nx && mx <= nx + nw && my >= ny && my <= ny + NOTE_HEIGHT) {
        if (tool === 'erase') {
          setNotes(prev => prev.filter(n => n.index !== note.index))
          return
        }
        // Resize handle check (last 6px)
        if (mx >= nx + nw - 6) {
          dragRef.current = {
            mode: 'resize', noteIndex: note.index,
            startX: e.clientX, startY: e.clientY,
            origTick: note.startTick, origPitch: note.pitch,
            origDuration: note.durationTicks,
          }
        } else {
          dragRef.current = {
            mode: 'move', noteIndex: note.index,
            startX: e.clientX, startY: e.clientY,
            origTick: note.startTick, origPitch: note.pitch,
            origDuration: note.durationTicks,
          }
        }
        setSelectedNotes(new Set([note.index]))
        return
      }
    }

    // Draw new note
    if (tool === 'draw' && pitch >= 0 && pitch < 128) {
      const snappedTick = snapTick(tick)
      const newNote: Note = {
        index: notes.length,
        startTick: snappedTick,
        durationTicks: snap,
        pitch,
        velocity: 0.8,
        muted: false,
      }
      setNotes(prev => [...prev, newNote])
      setSelectedNotes(new Set([newNote.index]))
      dragRef.current = {
        mode: 'draw', noteIndex: newNote.index,
        startX: e.clientX, startY: e.clientY,
        origTick: snappedTick, origPitch: pitch,
        origDuration: snap,
      }
    } else if (tool === 'select') {
      setSelectedNotes(new Set())
    }
  }, [notes, tool, snap, scrollX, scrollY, pixelsPerTick])

  const handleMouseMove = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    const drag = dragRef.current
    if (drag.mode === 'none') return

    const dx = e.clientX - drag.startX
    const dy = e.clientY - drag.startY

    if (drag.mode === 'move') {
      const tickDelta = snapTick(dx / pixelsPerTick)
      const pitchDelta = -Math.round(dy / NOTE_HEIGHT)
      setNotes(prev => prev.map(n =>
        n.index === drag.noteIndex
          ? { ...n, startTick: Math.max(0, drag.origTick + tickDelta), pitch: Math.max(0, Math.min(127, drag.origPitch + pitchDelta)) }
          : n
      ))
    } else if (drag.mode === 'resize' || drag.mode === 'draw') {
      const tickDelta = snapTick(dx / pixelsPerTick)
      const newDur = Math.max(snap, drag.origDuration + tickDelta)
      setNotes(prev => prev.map(n =>
        n.index === drag.noteIndex ? { ...n, durationTicks: newDur } : n
      ))
    }
  }, [pixelsPerTick, snap])

  const handleMouseUp = useCallback(() => {
    dragRef.current.mode = 'none'
  }, [])

  // Scroll handling
  const handleWheel = useCallback((e: React.WheelEvent) => {
    if (e.ctrlKey || e.metaKey) {
      // Zoom horizontal
      e.preventDefault()
      setPixelsPerTick(prev => Math.max(0.02, Math.min(1, prev * (e.deltaY < 0 ? 1.15 : 0.87))))
    } else if (e.shiftKey) {
      setScrollX(prev => Math.max(0, prev + e.deltaY))
    } else {
      setScrollY(prev => Math.max(0, Math.min(totalHeight - 200, prev + e.deltaY)))
    }
  }, [totalHeight])

  return (
    <div ref={containerRef} style={{
      flex: 1, display: 'flex', flexDirection: 'column',
      background: hw.bgPanel, overflow: 'hidden',
    }}>
      {/* Piano Roll header bar */}
      <div style={{
        height: RULER_HEIGHT, background: hw.bgDeep,
        borderBottom: `1px solid ${hw.borderDark}`,
        display: 'flex', alignItems: 'center', padding: '0 8px', gap: 6,
      }}>
        <span style={{ fontSize: 10, fontWeight: 600, color: hw.textMuted }}>Piano Roll</span>
        <span style={{ fontSize: 9, color: hw.textFaint }}>
          {trackId && clipId ? `${clipId.slice(0, 8)}…` : 'No clip selected'}
        </span>
        <div style={{ flex: 1 }} />

        {/* Tool selector */}
        <div style={{ display: 'flex', gap: 1 }}>
          {(['draw', 'select', 'erase'] as const).map(t => (
            <button key={t} onClick={() => setTool(t)} style={{
              padding: '1px 6px', fontSize: 9, fontWeight: 600,
              color: tool === t ? '#FFF' : '#888',
              background: tool === t ? '#555' : 'transparent',
              border: `1px solid ${tool === t ? 'rgba(255,255,255,0.12)' : 'transparent'}`,
              borderRadius: 2, textTransform: 'uppercase',
            }}>
              {t}
            </button>
          ))}
        </div>

        {/* Snap selector */}
        <select
          value={snap}
          onChange={e => setSnap(Number(e.target.value))}
          style={{
            fontSize: 9, background: hw.bgInput, color: hw.textMuted,
            border: `1px solid ${hw.border}`, borderRadius: 2, padding: '1px 4px',
          }}
        >
          <option value={PPQ * 4}>Bar</option>
          <option value={PPQ * 2}>1/2</option>
          <option value={PPQ}>1/4</option>
          <option value={PPQ / 2}>1/8</option>
          <option value={PPQ / 4}>1/16</option>
          <option value={PPQ / 8}>1/32</option>
          <option value={PPQ / 3}>1/8T</option>
          <option value={PPQ / 6}>1/16T</option>
        </select>
      </div>

      {/* Main area: keyboard + grid */}
      <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
        {/* Piano keyboard */}
        <PianoKeyboard
          width={KEYBOARD_WIDTH}
          noteHeight={NOTE_HEIGHT}
          scrollY={scrollY}
          totalNotes={TOTAL_NOTES}
        />

        {/* Grid canvas */}
        <div style={{ flex: 1, position: 'relative', overflow: 'hidden' }}>
          {/* Time ruler */}
          <canvas
            style={{ display: 'block' }}
            ref={useRef<HTMLCanvasElement>(null)}
          />

          {/* Note grid */}
          <canvas
            ref={canvasRef}
            onMouseDown={handleMouseDown}
            onMouseMove={handleMouseMove}
            onMouseUp={handleMouseUp}
            onMouseLeave={handleMouseUp}
            onWheel={handleWheel}
            style={{ display: 'block', cursor: tool === 'draw' ? 'crosshair' : tool === 'erase' ? 'not-allowed' : 'default' }}
          />
        </div>
      </div>

      {/* Velocity lane */}
      <VelocityLane
        notes={notes}
        selectedNotes={selectedNotes}
        height={VELOCITY_LANE_HEIGHT}
        keyboardWidth={KEYBOARD_WIDTH}
        scrollX={scrollX}
        pixelsPerTick={pixelsPerTick}
        onVelocityChange={(index, vel) => {
          setNotes(prev => prev.map(n => n.index === index ? { ...n, velocity: vel } : n))
        }}
      />
    </div>
  )
}
