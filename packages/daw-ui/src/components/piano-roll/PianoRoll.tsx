import { useEffect, useRef, useState, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'
import { PianoKeyboard } from './PianoKeyboard'
import { VelocityLane } from './VelocityLane'
import { DetachButton } from '../FloatingWindow'
import { useTrackStore } from '../../stores/trackStore'
import { useProjectStore } from '../../stores/projectStore'

const PPQ = 960
const NOTE_HEIGHT = 14
const RULER_HEIGHT = 22
const KEYBOARD_WIDTH = 60
const VELOCITY_LANE_HEIGHT = 80
const TOTAL_NOTES = 128
const DEFAULT_SNAP = PPQ / 4

interface Note {
  index: number
  startTick: number
  durationTicks: number
  pitch: number
  velocity: number
  muted: boolean
}

interface MidiNoteInfo {
  index: number
  start_tick: number
  duration_ticks: number
  pitch: number
  velocity: number
  channel: number
  muted: boolean
}

function toNote(n: MidiNoteInfo): Note {
  return {
    index: n.index,
    startTick: n.start_tick,
    durationTicks: n.duration_ticks,
    pitch: n.pitch,
    velocity: n.velocity,
    muted: n.muted,
  }
}

export function PianoRoll() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const activeTrackId = useTrackStore(s => s.activeMidiTrackId)
  const activeClipId = useTrackStore(s => s.activeMidiClipId)
  const [notes, setNotes] = useState<Note[]>([])
  const [scrollX, setScrollX] = useState(0)
  const [scrollY, setScrollY] = useState(NOTE_HEIGHT * 60)
  const [pixelsPerTick, setPixelsPerTick] = useState(0.12)
  const [snap, setSnap] = useState(DEFAULT_SNAP)
  const [selectedNotes, setSelectedNotes] = useState<Set<number>>(new Set())
  const [tool, setTool] = useState<'draw' | 'select' | 'erase'>('draw')
  const dragRef = useRef<{
    mode: 'none' | 'draw' | 'move' | 'resize'
    noteIndex: number
    startX: number
    startY: number
    origTick: number
    origPitch: number
    origDuration: number
    committed: boolean
  }>({ mode: 'none', noteIndex: -1, startX: 0, startY: 0, origTick: 0, origPitch: 0, origDuration: 0, committed: false })

  const totalHeight = TOTAL_NOTES * NOTE_HEIGHT

  const snapTick = (tick: number) => Math.round(tick / snap) * snap
  const pitchFromY = (y: number) => TOTAL_NOTES - 1 - Math.floor((y + scrollY) / NOTE_HEIGHT)
  const tickFromX = (x: number) => (x - KEYBOARD_WIDTH + scrollX) / pixelsPerTick
  const yFromPitch = (pitch: number) => (TOTAL_NOTES - 1 - pitch) * NOTE_HEIGHT - scrollY
  const xFromTick = (tick: number) => tick * pixelsPerTick - scrollX + KEYBOARD_WIDTH

  const noteName = (pitch: number) => {
    const names = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
    return `${names[pitch % 12]}${Math.floor(pitch / 12) - 1}`
  }

  const isBlackKey = (pitch: number) => [1, 3, 6, 8, 10].includes(pitch % 12)

  const refreshNotes = useCallback(async () => {
    if (!activeTrackId || !activeClipId) { setNotes([]); return }
    try {
      const data = await invoke<MidiNoteInfo[]>('get_midi_notes', {
        trackId: activeTrackId,
        clipId: activeClipId,
      })
      setNotes(data.map(toNote))
    } catch (e) {
      console.warn('get_midi_notes failed', e)
      setNotes([])
    }
  }, [activeTrackId, activeClipId])

  useEffect(() => {
    refreshNotes()
    setSelectedNotes(new Set())
  }, [refreshNotes])

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

    ctx.fillStyle = '#0a0a0f'
    ctx.fillRect(0, 0, w, h)

    for (let pitch = 0; pitch < TOTAL_NOTES; pitch++) {
      const y = yFromPitch(pitch)
      if (y + NOTE_HEIGHT < 0 || y > h) continue
      if (isBlackKey(pitch)) {
        ctx.fillStyle = '#08080d'
        ctx.fillRect(0, y, w, NOTE_HEIGHT)
      }
      if (pitch % 12 === 0) {
        ctx.fillStyle = 'rgba(220,38,38,0.02)'
        ctx.fillRect(0, y, w, NOTE_HEIGHT)
      }
    }

    ctx.strokeStyle = 'rgba(255, 255, 255, 0.02)'
    ctx.lineWidth = 0.5
    for (let pitch = 0; pitch < TOTAL_NOTES; pitch++) {
      const y = yFromPitch(pitch) + NOTE_HEIGHT
      if (y < 0 || y > h) continue
      ctx.beginPath()
      ctx.moveTo(0, y)
      ctx.lineTo(w, y)
      ctx.stroke()
    }

    const startTick = Math.max(0, Math.floor(scrollX / pixelsPerTick / PPQ) * PPQ)
    const endTick = (scrollX + w) / pixelsPerTick

    for (let tick = startTick; tick <= endTick; tick += PPQ / 4) {
      const x = xFromTick(tick) - KEYBOARD_WIDTH
      if (x < 0 || x > w) continue

      const isBar = tick % (PPQ * 4) === 0
      const isBeat = tick % PPQ === 0

      if (isBar) {
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.08)'
        ctx.lineWidth = 1
      } else if (isBeat) {
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.04)'
        ctx.lineWidth = 0.5
      } else {
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.015)'
        ctx.lineWidth = 0.5
      }

      ctx.beginPath()
      ctx.moveTo(x, 0)
      ctx.lineTo(x, h)
      ctx.stroke()
    }

    for (const note of notes) {
      const x = xFromTick(note.startTick) - KEYBOARD_WIDTH
      const y = yFromPitch(note.pitch)
      const noteW = note.durationTicks * pixelsPerTick
      if (x + noteW < 0 || x > w || y + NOTE_HEIGHT < 0 || y > h) continue

      const isSelected = selectedNotes.has(note.index)
      const color = note.muted ? '#52525b' : '#DC2626'

      ctx.fillStyle = isSelected ? '#EF4444' : color
      ctx.globalAlpha = note.muted ? 0.4 : 0.85
      ctx.beginPath()
      ctx.roundRect(x + 0.5, y + 1, Math.max(noteW - 1, 2), NOTE_HEIGHT - 2, 4)
      ctx.fill()
      ctx.globalAlpha = 1

      if (!note.muted && !isSelected) {
        ctx.shadowColor = 'rgba(220,38,38,0.3)'
        ctx.shadowBlur = 4
      }

      ctx.strokeStyle = isSelected ? '#fff' : 'rgba(255,255,255,0.1)'
      ctx.lineWidth = isSelected ? 1.5 : 0.5
      ctx.beginPath()
      ctx.roundRect(x + 0.5, y + 1, Math.max(noteW - 1, 2), NOTE_HEIGHT - 2, 4)
      ctx.stroke()
      ctx.shadowBlur = 0

      if (noteW > 30) {
        ctx.fillStyle = isSelected ? '#fff' : 'rgba(255,255,255,0.7)'
        ctx.font = '9px Inter, ui-sans-serif, sans-serif'
        ctx.fillText(noteName(note.pitch), x + 4, y + NOTE_HEIGHT - 3)
      }

      const velH = (NOTE_HEIGHT - 4) * note.velocity
      ctx.fillStyle = `rgba(255, 255, 255, ${0.1 + note.velocity * 0.2})`
      ctx.fillRect(x + 1, y + NOTE_HEIGHT - 1 - velH, 2, velH)

      ctx.fillStyle = 'rgba(255, 255, 255, 0.2)'
      ctx.fillRect(x + noteW - 4, y + 3, 2, NOTE_HEIGHT - 6)
    }
  }, [notes, scrollX, scrollY, pixelsPerTick, selectedNotes])

  useEffect(() => { draw() }, [draw])

  useEffect(() => {
    const obs = new ResizeObserver(() => draw())
    if (containerRef.current) obs.observe(containerRef.current)
    return () => obs.disconnect()
  }, [draw])

  const handleMouseDown = useCallback(async (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!activeTrackId || !activeClipId) return
    const rect = canvasRef.current!.getBoundingClientRect()
    const mx = e.clientX - rect.left
    const my = e.clientY - rect.top
    const tick = tickFromX(mx + KEYBOARD_WIDTH)
    const pitch = pitchFromY(my + RULER_HEIGHT)

    for (const note of notes) {
      const nx = xFromTick(note.startTick) - KEYBOARD_WIDTH
      const ny = yFromPitch(note.pitch)
      const nw = note.durationTicks * pixelsPerTick

      if (mx >= nx && mx <= nx + nw && my >= ny && my <= ny + NOTE_HEIGHT) {
        if (tool === 'erase') {
          try {
            await invoke('delete_midi_note', {
              trackId: activeTrackId,
              clipId: activeClipId,
              noteIndex: note.index,
            })
            useProjectStore.getState().markDirty()
            await refreshNotes()
          } catch (err) { console.warn('delete_midi_note failed', err) }
          return
        }
        const resizing = mx >= nx + nw - 6
        dragRef.current = {
          mode: resizing ? 'resize' : 'move',
          noteIndex: note.index,
          startX: e.clientX, startY: e.clientY,
          origTick: note.startTick, origPitch: note.pitch,
          origDuration: note.durationTicks,
          committed: false,
        }
        if (e.shiftKey) {
          setSelectedNotes(prev => {
            const next = new Set(prev)
            if (next.has(note.index)) next.delete(note.index)
            else next.add(note.index)
            return next
          })
        } else {
          setSelectedNotes(new Set([note.index]))
        }
        return
      }
    }

    if (tool === 'draw' && pitch >= 0 && pitch < 128) {
      const snappedTick = Math.max(0, snapTick(tick))
      try {
        const newIndex = await invoke<number>('add_midi_note', {
          trackId: activeTrackId,
          clipId: activeClipId,
          pitch,
          startTick: snappedTick,
          durationTicks: snap,
          velocity: 0.8,
        })
        useProjectStore.getState().markDirty()
        const draftNote: Note = {
          index: newIndex,
          startTick: snappedTick,
          durationTicks: snap,
          pitch,
          velocity: 0.8,
          muted: false,
        }
        setNotes(prev => [...prev, draftNote])
        setSelectedNotes(new Set([newIndex]))
        dragRef.current = {
          mode: 'draw', noteIndex: newIndex,
          startX: e.clientX, startY: e.clientY,
          origTick: snappedTick, origPitch: pitch,
          origDuration: snap,
          committed: false,
        }
      } catch (err) { console.warn('add_midi_note failed', err) }
    } else if (tool === 'select') {
      setSelectedNotes(new Set())
    }
  }, [notes, tool, snap, scrollX, scrollY, pixelsPerTick, activeTrackId, activeClipId, refreshNotes])

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

  const handleMouseUp = useCallback(async () => {
    const drag = dragRef.current
    if (drag.mode === 'none' || drag.committed) {
      dragRef.current.mode = 'none'
      return
    }
    drag.committed = true
    const note = notes.find(n => n.index === drag.noteIndex)
    dragRef.current.mode = 'none'
    if (!note || !activeTrackId || !activeClipId) return

    const changed =
      note.startTick !== drag.origTick ||
      note.pitch !== drag.origPitch ||
      note.durationTicks !== drag.origDuration
    if (!changed && drag.mode !== 'draw') return

    try {
      await invoke('update_midi_note', {
        trackId: activeTrackId,
        clipId: activeClipId,
        noteIndex: drag.noteIndex,
        pitch: note.pitch,
        startTick: note.startTick,
        durationTicks: note.durationTicks,
      })
      useProjectStore.getState().markDirty()
      await refreshNotes()
    } catch (err) { console.warn('update_midi_note failed', err) }
  }, [notes, activeTrackId, activeClipId, refreshNotes])

  const handleVelocityChange = useCallback(async (noteIndex: number, velocity: number) => {
    if (!activeTrackId || !activeClipId) return
    setNotes(prev => prev.map(n => n.index === noteIndex ? { ...n, velocity } : n))
    try {
      await invoke('update_midi_note', {
        trackId: activeTrackId,
        clipId: activeClipId,
        noteIndex,
        velocity,
      })
      useProjectStore.getState().markDirty()
    } catch (err) { console.warn('update_midi_note velocity failed', err) }
  }, [activeTrackId, activeClipId])

  const handleWheel = useCallback((e: React.WheelEvent) => {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault()
      setPixelsPerTick(prev => Math.max(0.02, Math.min(1, prev * (e.deltaY < 0 ? 1.15 : 0.87))))
    } else if (e.shiftKey) {
      setScrollX(prev => Math.max(0, prev + e.deltaY))
    } else {
      setScrollY(prev => Math.max(0, Math.min(totalHeight - 200, prev + e.deltaY)))
    }
  }, [totalHeight])

  const handleKeyDown = useCallback(async (e: KeyboardEvent) => {
    if (!activeTrackId || !activeClipId) return
    if (selectedNotes.size === 0) return
    const target = e.target as HTMLElement
    if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) return

    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
      e.preventDefault()
      setSelectedNotes(new Set(notes.map(n => n.index)))
      return
    }

    if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault()
      const indicesDesc = Array.from(selectedNotes).sort((a, b) => b - a)
      try {
        for (const idx of indicesDesc) {
          await invoke('delete_midi_note', {
            trackId: activeTrackId,
            clipId: activeClipId,
            noteIndex: idx,
          })
        }
        useProjectStore.getState().markDirty()
        setSelectedNotes(new Set())
        await refreshNotes()
      } catch (err) { console.warn('delete selected failed', err) }
    }
  }, [activeTrackId, activeClipId, selectedNotes, refreshNotes, notes])

  useEffect(() => {
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [handleKeyDown])

  const emptyHint = !activeTrackId || !activeClipId
    ? 'Select a MIDI track or double-click a MIDI clip to edit notes.'
    : null

  return (
    <div ref={containerRef} style={{
      flex: 1, display: 'flex', flexDirection: 'column',
      background: 'rgba(255,255,255,0.02)', backdropFilter: hw.blur.sm, overflow: 'hidden',
    }}>
      <div style={{
        height: RULER_HEIGHT, background: 'rgba(255,255,255,0.01)',
        borderBottom: `1px solid ${hw.border}`,
        display: 'flex', alignItems: 'center', padding: '0 8px', gap: 6,
      }}>
        <span style={{ fontSize: 10, fontWeight: 600, color: hw.textMuted }}>Piano Roll</span>
        <span style={{ fontSize: 9, color: hw.textFaint }}>
          {activeTrackId && activeClipId ? `${activeClipId.slice(0, 8)}…` : 'No clip selected'}
        </span>
        <div style={{ flex: 1 }} />

        <div style={{ display: 'flex', gap: 1 }}>
          {(['draw', 'select', 'erase'] as const).map(t => (
            <button key={t} onClick={() => setTool(t)} style={{
              padding: '1px 6px', fontSize: 9, fontWeight: 600,
              color: tool === t ? hw.accent : hw.textFaint,
              background: tool === t ? hw.accentDim : 'transparent',
              border: `1px solid ${tool === t ? hw.accentGlow : 'transparent'}`,
              borderRadius: hw.radius.sm, textTransform: 'uppercase',
              transition: 'all 0.1s',
            }}>
              {t}
            </button>
          ))}
        </div>

        <select
          value={snap}
          onChange={e => setSnap(Number(e.target.value))}
          style={{
            fontSize: 9, background: 'rgba(255,255,255,0.04)', color: hw.textMuted,
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, padding: '1px 4px',
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
        <DetachButton panelId="pianoRoll" />
      </div>

      <div style={{ flex: 1, display: 'flex', overflow: 'hidden', position: 'relative' }}>
        <PianoKeyboard
          width={KEYBOARD_WIDTH}
          noteHeight={NOTE_HEIGHT}
          scrollY={scrollY}
          totalNotes={TOTAL_NOTES}
        />

        <div style={{ flex: 1, position: 'relative', overflow: 'hidden' }}>
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

        {emptyHint && (
          <div style={{
            position: 'absolute', inset: 0, display: 'flex',
            alignItems: 'center', justifyContent: 'center',
            pointerEvents: 'none', color: hw.textFaint, fontSize: 12,
          }}>
            {emptyHint}
          </div>
        )}
      </div>

      <VelocityLane
        notes={notes}
        selectedNotes={selectedNotes}
        height={VELOCITY_LANE_HEIGHT}
        keyboardWidth={KEYBOARD_WIDTH}
        scrollX={scrollX}
        pixelsPerTick={pixelsPerTick}
        onVelocityChange={handleVelocityChange}
      />
    </div>
  )
}
