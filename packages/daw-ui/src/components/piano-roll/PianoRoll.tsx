import { useEffect, useRef, useState, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'
import { PianoKeyboard } from './PianoKeyboard'
import { VelocityLane } from './VelocityLane'
import { Minimap } from './Minimap'
import { DetachButton } from '../FloatingWindow'
import { useTrackStore } from '../../stores/trackStore'
import { useProjectStore } from '../../stores/projectStore'
import { useTransportStore } from '../../stores/transportStore'

const PPQ = 960
const MINIMAP_HEIGHT = 36
const DEFAULT_NOTE_HEIGHT = 14
const MIN_NOTE_HEIGHT = 6
const MAX_NOTE_HEIGHT = 40
const RULER_HEIGHT = 22
const KEYBOARD_WIDTH = 60
const VELOCITY_LANE_HEIGHT = 80
const TOTAL_NOTES = 128
const DEFAULT_SNAP = PPQ / 4

const SCALE_TYPES: Record<string, { name: string; intervals: number[] }> = {
  chromatic: { name: 'Chromatic', intervals: [0,1,2,3,4,5,6,7,8,9,10,11] },
  major:     { name: 'Major',     intervals: [0,2,4,5,7,9,11] },
  minor:     { name: 'Natural Minor', intervals: [0,2,3,5,7,8,10] },
  harmonic:  { name: 'Harmonic Minor', intervals: [0,2,3,5,7,8,11] },
  melodic:   { name: 'Melodic Minor', intervals: [0,2,3,5,7,9,11] },
  dorian:    { name: 'Dorian',    intervals: [0,2,3,5,7,9,10] },
  phrygian:  { name: 'Phrygian',  intervals: [0,1,3,5,7,8,10] },
  lydian:    { name: 'Lydian',    intervals: [0,2,4,6,7,9,11] },
  mixolydian:{ name: 'Mixolydian', intervals: [0,2,4,5,7,9,10] },
  locrian:   { name: 'Locrian',   intervals: [0,1,3,5,6,8,10] },
  pentMajor: { name: 'Pentatonic Major', intervals: [0,2,4,7,9] },
  pentMinor: { name: 'Pentatonic Minor', intervals: [0,3,5,7,10] },
  blues:     { name: 'Blues',     intervals: [0,3,5,6,7,10] },
  wholeTone: { name: 'Whole Tone', intervals: [0,2,4,6,8,10] },
  hungarian: { name: 'Hungarian Minor', intervals: [0,2,3,6,7,8,11] },
}

function isPitchInScale(pitch: number, root: number, type: keyof typeof SCALE_TYPES) {
  const rel = ((pitch - root) % 12 + 12) % 12
  return SCALE_TYPES[type].intervals.includes(rel)
}

function snapPitchToScale(pitch: number, root: number, type: keyof typeof SCALE_TYPES) {
  if (type === 'chromatic') return pitch
  const intervals = SCALE_TYPES[type].intervals
  const octave = Math.floor((pitch - root) / 12)
  const rel = ((pitch - root) % 12 + 12) % 12
  let best = intervals[0]
  let bestDist = 12
  for (const iv of intervals) {
    const d = Math.abs(rel - iv)
    if (d < bestDist) { bestDist = d; best = iv }
  }
  return root + octave * 12 + best
}

const NOTE_NAMES_SHARP = ['C','C#','D','D#','E','F','F#','G','G#','A','A#','B']

const CHORD_TYPES: Record<string, { name: string; intervals: number[] }> = {
  maj:      { name: 'Major',       intervals: [0, 4, 7] },
  min:      { name: 'Minor',       intervals: [0, 3, 7] },
  dim:      { name: 'Diminished',  intervals: [0, 3, 6] },
  aug:      { name: 'Augmented',   intervals: [0, 4, 8] },
  maj7:     { name: 'Maj 7',       intervals: [0, 4, 7, 11] },
  min7:     { name: 'Min 7',       intervals: [0, 3, 7, 10] },
  dom7:     { name: 'Dom 7',       intervals: [0, 4, 7, 10] },
  dim7:     { name: 'Dim 7',       intervals: [0, 3, 6, 9] },
  halfDim7: { name: 'Half-dim 7',  intervals: [0, 3, 6, 10] },
  sus2:     { name: 'Sus 2',       intervals: [0, 2, 7] },
  sus4:     { name: 'Sus 4',       intervals: [0, 5, 7] },
  add9:     { name: 'Add 9',       intervals: [0, 4, 7, 14] },
  min9:     { name: 'Min 9',       intervals: [0, 3, 7, 10, 14] },
  maj9:     { name: 'Maj 9',       intervals: [0, 4, 7, 11, 14] },
  power:    { name: 'Power (5)',   intervals: [0, 7] },
  custom:   { name: 'Custom…',     intervals: [0, 4, 7] },
}

const INTERVAL_LABELS = ['R', 'b2', '2', 'b3', '3', '4', 'b5', '5', 'b6', '6', 'b7', '7']

function applyInversion(intervals: number[], inv: number): number[] {
  if (inv <= 0 || intervals.length < 2) return intervals
  const n = Math.min(inv, intervals.length - 1)
  const result = intervals.slice()
  for (let i = 0; i < n; i++) {
    const first = result.shift()!
    result.push(first + 12)
  }
  return result
}

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
  const tracks = useTrackStore(s => s.tracks)
  const [notes, setNotes] = useState<Note[]>([])
  const [ghostMode, setGhostMode] = useState<'off' | 'track' | 'all'>('off')
  const [ghostNotes, setGhostNotes] = useState<Array<Note & { color: string; trackId: string; clipId: string }>>([])
  const [showMinimap, setShowMinimap] = useState(false)
  const [viewportWidth, setViewportWidth] = useState(800)
  const [scrollX, setScrollX] = useState(0)
  const [noteHeight, setNoteHeight] = useState(DEFAULT_NOTE_HEIGHT)
  const [scrollY, setScrollY] = useState(DEFAULT_NOTE_HEIGHT * 60)
  const [followPlayhead, setFollowPlayhead] = useState(false)
  const [pixelsPerTick, setPixelsPerTick] = useState(0.12)
  const [snap, setSnap] = useState(DEFAULT_SNAP)
  const [selectedNotes, setSelectedNotes] = useState<Set<number>>(new Set())
  const [tool, setTool] = useState<'draw' | 'select' | 'erase' | 'chord'>('draw')
  const [chordType, setChordType] = useState<keyof typeof CHORD_TYPES>('maj')
  const [chordInversion, setChordInversion] = useState<number>(0)
  const [customChordSet, setCustomChordSet] = useState<Set<number>>(new Set([0, 4, 7]))
  const [scaleRoot, setScaleRoot] = useState<number>(0)
  const [scaleType, setScaleType] = useState<keyof typeof SCALE_TYPES>('chromatic')
  const [snapToScale, setSnapToScale] = useState(false)
  const [marquee, setMarquee] = useState<{ x1: number; y1: number; x2: number; y2: number } | null>(null)
  const marqueeRef = useRef<{ x1: number; y1: number; x2: number; y2: number; additive: boolean } | null>(null)
  const [hoverInfo, setHoverInfo] = useState<{ x: number; y: number; label: string } | null>(null)
  const [chordPreview, setChordPreview] = useState<{ rootPitch: number; startTick: number } | null>(null)
  const [noteCtx, setNoteCtx] = useState<{ x: number; y: number; noteIndex: number } | null>(null)
  const [velFromY, setVelFromY] = useState(false)
  const clipboardRef = useRef<Note[]>([])
  const focusedRef = useRef(false)
  const dragRef = useRef<{
    mode: 'none' | 'draw' | 'move' | 'resize' | 'resizeLeft'
    noteIndex: number
    startX: number
    startY: number
    origTick: number
    origPitch: number
    origDuration: number
    freeMove: boolean
    committed: boolean
  }>({ mode: 'none', noteIndex: -1, startX: 0, startY: 0, origTick: 0, origPitch: 0, origDuration: 0, freeMove: false, committed: false })

  const totalHeight = TOTAL_NOTES * noteHeight

  const snapTick = (tick: number) => Math.round(tick / snap) * snap
  const pitchFromY = (y: number) => TOTAL_NOTES - 1 - Math.floor((y + scrollY) / noteHeight)
  const tickFromX = (x: number) => (x - KEYBOARD_WIDTH + scrollX) / pixelsPerTick
  const yFromPitch = (pitch: number) => (TOTAL_NOTES - 1 - pitch) * noteHeight - scrollY
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

  useEffect(() => {
    let cancelled = false
    async function fetchGhosts() {
      if (ghostMode === 'off' || !activeTrackId || !activeClipId) {
        if (!cancelled) setGhostNotes([])
        return
      }
      const candidates: Array<{ trackId: string; clipId: string; color: string }> = []
      for (const t of tracks) {
        if (ghostMode === 'track' && t.id !== activeTrackId) continue
        for (const c of t.clips) {
          if (c.kind !== 'midi') continue
          if (t.id === activeTrackId && c.id === activeClipId) continue
          candidates.push({ trackId: t.id, clipId: c.id, color: t.color || '#64748b' })
        }
      }
      const collected: Array<Note & { color: string; trackId: string; clipId: string }> = []
      for (const { trackId, clipId, color } of candidates) {
        try {
          const data = await invoke<MidiNoteInfo[]>('get_midi_notes', { trackId, clipId })
          for (const n of data) {
            collected.push({
              index: -1,
              startTick: n.start_tick,
              durationTicks: n.duration_ticks,
              pitch: n.pitch,
              velocity: n.velocity,
              muted: n.muted,
              color,
              trackId,
              clipId,
            })
          }
        } catch {}
      }
      if (!cancelled) setGhostNotes(collected)
    }
    fetchGhosts()
    return () => { cancelled = true }
  }, [ghostMode, activeTrackId, activeClipId, tracks, notes])

  const draw = useCallback(() => {
    const canvas = canvasRef.current
    const container = containerRef.current
    if (!canvas || !container) return

    const rect = container.getBoundingClientRect()
    const w = rect.width - KEYBOARD_WIDTH
    const h = rect.height - RULER_HEIGHT - VELOCITY_LANE_HEIGHT
    if (Math.abs(w - viewportWidth) > 1) setViewportWidth(w)
    canvas.width = w * devicePixelRatio
    canvas.height = h * devicePixelRatio
    canvas.style.width = `${w}px`
    canvas.style.height = `${h}px`

    const ctx = canvas.getContext('2d')!
    ctx.scale(devicePixelRatio, devicePixelRatio)

    ctx.fillStyle = '#0a0a0f'
    ctx.fillRect(0, 0, w, h)

    const highlightScale = scaleType !== 'chromatic'
    for (let pitch = 0; pitch < TOTAL_NOTES; pitch++) {
      const y = yFromPitch(pitch)
      if (y + noteHeight < 0 || y > h) continue
      if (isBlackKey(pitch)) {
        ctx.fillStyle = '#08080d'
        ctx.fillRect(0, y, w, noteHeight)
      }
      if (pitch % 12 === 0) {
        ctx.fillStyle = 'rgba(220,38,38,0.02)'
        ctx.fillRect(0, y, w, noteHeight)
      }
      if (highlightScale) {
        const inScale = isPitchInScale(pitch, scaleRoot, scaleType)
        if (!inScale) {
          ctx.fillStyle = 'rgba(0,0,0,0.35)'
          ctx.fillRect(0, y, w, noteHeight)
        } else if (((pitch - scaleRoot) % 12 + 12) % 12 === 0) {
          ctx.fillStyle = 'rgba(220,38,38,0.05)'
          ctx.fillRect(0, y, w, noteHeight)
        }
      }
    }

    ctx.strokeStyle = 'rgba(255, 255, 255, 0.02)'
    ctx.lineWidth = 0.5
    for (let pitch = 0; pitch < TOTAL_NOTES; pitch++) {
      const y = yFromPitch(pitch) + noteHeight
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

    if (ghostMode !== 'off' && ghostNotes.length > 0) {
      const highlight = scaleType !== 'chromatic'
      ctx.save()
      for (const note of ghostNotes) {
        const x = xFromTick(note.startTick) - KEYBOARD_WIDTH
        const y = yFromPitch(note.pitch)
        const noteW = note.durationTicks * pixelsPerTick
        if (x + noteW < 0 || x > w || y + noteHeight < 0 || y > h) continue
        const inScale = !highlight || isPitchInScale(note.pitch, scaleRoot, scaleType)
        ctx.globalAlpha = inScale ? 0.22 : 0.08
        ctx.fillStyle = note.color
        ctx.beginPath()
        ctx.roundRect(x + 0.5, y + 1, Math.max(noteW - 1, 2), noteHeight - 2, 4)
        ctx.fill()
        ctx.globalAlpha = inScale ? 0.4 : 0.18
        ctx.strokeStyle = note.color
        ctx.lineWidth = 0.5
        ctx.stroke()
      }
      ctx.restore()
    }

    if (tool === 'chord' && chordPreview) {
      const baseIntervals = chordType === 'custom'
        ? Array.from(customChordSet).sort((a, b) => a - b)
        : CHORD_TYPES[chordType].intervals
      const intervals = applyInversion(baseIntervals.length > 0 ? baseIntervals : [0], chordInversion)
      const noteW = snap * pixelsPerTick
      ctx.save()
      ctx.setLineDash([3, 2])
      ctx.lineWidth = 1
      for (const iv of intervals) {
        const p = chordPreview.rootPitch + iv
        if (p < 0 || p > 127) continue
        const x = xFromTick(chordPreview.startTick) - KEYBOARD_WIDTH
        const y = yFromPitch(p)
        if (x + noteW < 0 || x > w || y + noteHeight < 0 || y > h) continue
        ctx.globalAlpha = 0.18
        ctx.fillStyle = hw.accent
        ctx.beginPath()
        ctx.roundRect(x + 0.5, y + 1, Math.max(noteW - 1, 2), noteHeight - 2, 4)
        ctx.fill()
        ctx.globalAlpha = 0.7
        ctx.strokeStyle = hw.accent
        ctx.beginPath()
        ctx.roundRect(x + 0.5, y + 1, Math.max(noteW - 1, 2), noteHeight - 2, 4)
        ctx.stroke()
      }
      ctx.restore()
    }

    for (const note of notes) {
      const x = xFromTick(note.startTick) - KEYBOARD_WIDTH
      const y = yFromPitch(note.pitch)
      const noteW = note.durationTicks * pixelsPerTick
      if (x + noteW < 0 || x > w || y + noteHeight < 0 || y > h) continue

      const isSelected = selectedNotes.has(note.index)
      let color = '#DC2626'
      if (note.muted) {
        color = '#52525b'
      } else {
        // Velocity-driven lightness: 0.2 → dark maroon, 1.0 → bright red
        const t = Math.max(0, Math.min(1, note.velocity))
        const r = Math.round(100 + 155 * t)
        const g = Math.round(20 + 38 * t)
        const b = Math.round(20 + 38 * t)
        color = `rgb(${r},${g},${b})`
      }

      ctx.fillStyle = isSelected ? '#EF4444' : color
      ctx.globalAlpha = note.muted ? 0.4 : 0.85
      ctx.beginPath()
      ctx.roundRect(x + 0.5, y + 1, Math.max(noteW - 1, 2), noteHeight - 2, 4)
      ctx.fill()
      ctx.globalAlpha = 1

      if (!note.muted && !isSelected) {
        ctx.shadowColor = 'rgba(220,38,38,0.3)'
        ctx.shadowBlur = 4
      }

      ctx.strokeStyle = isSelected ? '#fff' : 'rgba(255,255,255,0.1)'
      ctx.lineWidth = isSelected ? 1.5 : 0.5
      ctx.beginPath()
      ctx.roundRect(x + 0.5, y + 1, Math.max(noteW - 1, 2), noteHeight - 2, 4)
      ctx.stroke()
      ctx.shadowBlur = 0

      if (noteW > 30) {
        ctx.fillStyle = isSelected ? '#fff' : 'rgba(255,255,255,0.7)'
        ctx.font = '9px Inter, ui-sans-serif, sans-serif'
        ctx.fillText(noteName(note.pitch), x + 4, y + noteHeight - 3)
      }

      const velH = (noteHeight - 4) * note.velocity
      ctx.fillStyle = `rgba(255, 255, 255, ${0.1 + note.velocity * 0.2})`
      ctx.fillRect(x + 1, y + noteHeight - 1 - velH, 2, velH)

      ctx.fillStyle = 'rgba(255, 255, 255, 0.2)'
      ctx.fillRect(x + noteW - 4, y + 3, 2, noteHeight - 6)
    }

    if (marquee) {
      const mx = Math.min(marquee.x1, marquee.x2)
      const my = Math.min(marquee.y1, marquee.y2)
      const mw = Math.abs(marquee.x2 - marquee.x1)
      const mh = Math.abs(marquee.y2 - marquee.y1)
      ctx.fillStyle = 'rgba(220,38,38,0.10)'
      ctx.fillRect(mx, my, mw, mh)
      ctx.strokeStyle = 'rgba(239,68,68,0.85)'
      ctx.lineWidth = 1
      ctx.setLineDash([4, 3])
      ctx.strokeRect(mx + 0.5, my + 0.5, mw, mh)
      ctx.setLineDash([])
    }
  }, [notes, scrollX, scrollY, pixelsPerTick, selectedNotes, marquee, scaleRoot, scaleType, ghostMode, ghostNotes, noteHeight, chordPreview, tool, chordType, chordInversion, customChordSet, snap])

  useEffect(() => { draw() }, [draw])

  useEffect(() => {
    const obs = new ResizeObserver(() => draw())
    if (containerRef.current) obs.observe(containerRef.current)
    return () => obs.disconnect()
  }, [draw])

  const panRef = useRef<{ startX: number; startY: number; origScrollX: number; origScrollY: number } | null>(null)

  const handleMouseDown = useCallback(async (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (e.button === 1) {
      e.preventDefault()
      panRef.current = {
        startX: e.clientX, startY: e.clientY,
        origScrollX: scrollX, origScrollY: scrollY,
      }
      const onMove = (ev: MouseEvent) => {
        const p = panRef.current
        if (!p) return
        setScrollX(Math.max(0, p.origScrollX - (ev.clientX - p.startX)))
        setScrollY(Math.max(0, p.origScrollY - (ev.clientY - p.startY)))
      }
      const onUp = () => {
        panRef.current = null
        window.removeEventListener('mousemove', onMove)
        window.removeEventListener('mouseup', onUp)
      }
      window.addEventListener('mousemove', onMove)
      window.addEventListener('mouseup', onUp)
      return
    }
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

      if (mx >= nx && mx <= nx + nw && my >= ny && my <= ny + noteHeight) {
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
        const resizingRight = mx >= nx + nw - 6
        const resizingLeft = !resizingRight && mx <= nx + 6 && nw > 12
        const mode = resizingRight ? 'resize' : resizingLeft ? 'resizeLeft' : 'move'
        dragRef.current = {
          mode,
          noteIndex: note.index,
          startX: e.clientX, startY: e.clientY,
          origTick: note.startTick, origPitch: note.pitch,
          origDuration: note.durationTicks,
          freeMove: e.altKey,
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

    if (ghostMode !== 'off' && (tool === 'select' || e.altKey)) {
      for (const g of ghostNotes) {
        const gx = xFromTick(g.startTick) - KEYBOARD_WIDTH
        const gy = yFromPitch(g.pitch)
        const gw = g.durationTicks * pixelsPerTick
        if (mx >= gx && mx <= gx + gw && my >= gy && my <= gy + noteHeight) {
          useTrackStore.getState().setActiveMidiClip(g.trackId, g.clipId)
          return
        }
      }
    }

    if (tool === 'chord' && pitch >= 0 && pitch < 128) {
      const snappedTick = Math.max(0, snapTick(tick))
      const rootPitch = snapToScale && scaleType !== 'chromatic'
        ? snapPitchToScale(pitch, scaleRoot, scaleType)
        : pitch
      const baseIntervals = chordType === 'custom'
        ? Array.from(customChordSet).sort((a, b) => a - b)
        : CHORD_TYPES[chordType].intervals
      const intervals = applyInversion(baseIntervals.length > 0 ? baseIntervals : [0], chordInversion)
      const newIndices: number[] = []
      try {
        for (const iv of intervals) {
          const p = rootPitch + iv
          if (p < 0 || p > 127) continue
          const idx = await invoke<number>('add_midi_note', {
            trackId: activeTrackId,
            clipId: activeClipId,
            pitch: p,
            startTick: snappedTick,
            durationTicks: snap,
            velocity: 0.8,
          })
          newIndices.push(idx)
        }
        useProjectStore.getState().markDirty()
        await refreshNotes()
        setSelectedNotes(new Set(newIndices))
      } catch (err) { console.warn('chord stamp failed', err) }
      return
    }

    if (tool === 'draw' && pitch >= 0 && pitch < 128) {
      const snappedTick = Math.max(0, snapTick(tick))
      const drawPitch = snapToScale ? snapPitchToScale(pitch, scaleRoot, scaleType) : pitch
      let drawVelocity = 0.8
      if (velFromY) {
        const pitchY = yFromPitch(drawPitch)
        const within = Math.max(0, Math.min(1, (my - pitchY) / noteHeight))
        drawVelocity = Math.max(0.1, Math.min(1, 1 - within * 0.9))
      }
      try {
        const newIndex = await invoke<number>('add_midi_note', {
          trackId: activeTrackId,
          clipId: activeClipId,
          pitch: drawPitch,
          startTick: snappedTick,
          durationTicks: snap,
          velocity: drawVelocity,
        })
        useProjectStore.getState().markDirty()
        const draftNote: Note = {
          index: newIndex,
          startTick: snappedTick,
          durationTicks: snap,
          pitch: drawPitch,
          velocity: drawVelocity,
          muted: false,
        }
        setNotes(prev => [...prev, draftNote])
        setSelectedNotes(new Set([newIndex]))
        dragRef.current = {
          mode: 'draw', noteIndex: newIndex,
          startX: e.clientX, startY: e.clientY,
          origTick: snappedTick, origPitch: drawPitch,
          origDuration: snap,
          freeMove: e.altKey,
          committed: false,
        }
      } catch (err) { console.warn('add_midi_note failed', err) }
    } else if (tool === 'select') {
      marqueeRef.current = { x1: mx, y1: my, x2: mx, y2: my, additive: e.shiftKey }
      setMarquee({ x1: mx, y1: my, x2: mx, y2: my })
      if (!e.shiftKey) setSelectedNotes(new Set())
    }
  }, [notes, tool, snap, scrollX, scrollY, pixelsPerTick, activeTrackId, activeClipId, refreshNotes, snapToScale, scaleRoot, scaleType, chordType, chordInversion, customChordSet, noteHeight, ghostMode, ghostNotes, velFromY])

  const handleMouseMove = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    if (marqueeRef.current) {
      const rect = canvasRef.current!.getBoundingClientRect()
      const mx = e.clientX - rect.left
      const my = e.clientY - rect.top
      marqueeRef.current.x2 = mx
      marqueeRef.current.y2 = my
      setMarquee({
        x1: marqueeRef.current.x1, y1: marqueeRef.current.y1,
        x2: mx, y2: my,
      })
      return
    }

    const drag = dragRef.current
    if (drag.mode === 'none') {
      const rect = canvasRef.current!.getBoundingClientRect()
      const mx = e.clientX - rect.left
      const my = e.clientY - rect.top

      if (tool === 'chord') {
        const tick = tickFromX(mx + KEYBOARD_WIDTH)
        const pitch = pitchFromY(my + RULER_HEIGHT)
        if (pitch >= 0 && pitch < 128 && tick >= 0) {
          const snappedTick = Math.max(0, snapTick(tick))
          const rootPitch = snapToScale && scaleType !== 'chromatic'
            ? snapPitchToScale(pitch, scaleRoot, scaleType)
            : pitch
          setChordPreview({ rootPitch, startTick: snappedTick })
        } else if (chordPreview) {
          setChordPreview(null)
        }
      } else if (chordPreview) {
        setChordPreview(null)
      }

      let found: Note | null = null
      for (const note of notes) {
        const nx = xFromTick(note.startTick) - KEYBOARD_WIDTH
        const ny = yFromPitch(note.pitch)
        const nw = note.durationTicks * pixelsPerTick
        if (mx >= nx && mx <= nx + nw && my >= ny && my <= ny + noteHeight) {
          found = note
          break
        }
      }
      if (found) {
        const name = `${NOTE_NAMES_SHARP[found.pitch % 12]}${Math.floor(found.pitch / 12) - 1}`
        setHoverInfo({
          x: e.clientX - rect.left,
          y: e.clientY - rect.top,
          label: `${name} · v${Math.round(found.velocity * 127)} · ${found.durationTicks}t`,
        })
      } else if (hoverInfo) {
        setHoverInfo(null)
      }
      return
    }

    const dx = e.clientX - drag.startX
    const dy = e.clientY - drag.startY

    if (drag.mode === 'move') {
      const rawDelta = dx / pixelsPerTick
      const tickDelta = drag.freeMove ? Math.round(rawDelta) : snapTick(rawDelta)
      const pitchDelta = -Math.round(dy / noteHeight)
      setNotes(prev => prev.map(n =>
        n.index === drag.noteIndex
          ? { ...n, startTick: Math.max(0, drag.origTick + tickDelta), pitch: Math.max(0, Math.min(127, drag.origPitch + pitchDelta)) }
          : n
      ))
    } else if (drag.mode === 'resize' || drag.mode === 'draw') {
      const rawDelta = dx / pixelsPerTick
      const tickDelta = drag.freeMove ? Math.round(rawDelta) : snapTick(rawDelta)
      const newDur = Math.max(snap, drag.origDuration + tickDelta)
      if (drag.mode === 'draw') {
        // During draw, also allow pitch adjust via vertical drag
        const pitchDelta = -Math.round(dy / noteHeight)
        setNotes(prev => prev.map(n =>
          n.index === drag.noteIndex ? {
            ...n,
            durationTicks: newDur,
            pitch: Math.max(0, Math.min(127, drag.origPitch + pitchDelta)),
          } : n
        ))
      } else {
        setNotes(prev => prev.map(n =>
          n.index === drag.noteIndex ? { ...n, durationTicks: newDur } : n
        ))
      }
    } else if (drag.mode === 'resizeLeft') {
      const rawDelta = dx / pixelsPerTick
      const tickDelta = drag.freeMove ? Math.round(rawDelta) : snapTick(rawDelta)
      const newStart = Math.max(0, drag.origTick + tickDelta)
      const maxStart = drag.origTick + drag.origDuration - snap
      const clampedStart = Math.min(newStart, Math.max(0, maxStart))
      const newDur = drag.origDuration + (drag.origTick - clampedStart)
      setNotes(prev => prev.map(n =>
        n.index === drag.noteIndex ? { ...n, startTick: clampedStart, durationTicks: newDur } : n
      ))
    }
  }, [pixelsPerTick, snap, noteHeight, notes, scrollX, scrollY, hoverInfo])

  const handleMouseUp = useCallback(async () => {
    if (marqueeRef.current) {
      const m = marqueeRef.current
      const additive = m.additive
      marqueeRef.current = null
      const xLo = Math.min(m.x1, m.x2)
      const xHi = Math.max(m.x1, m.x2)
      const yLo = Math.min(m.y1, m.y2)
      const yHi = Math.max(m.y1, m.y2)
      setMarquee(null)
      if (Math.abs(xHi - xLo) < 3 && Math.abs(yHi - yLo) < 3) return
      const hit = new Set<number>()
      for (const note of notes) {
        const nx = xFromTick(note.startTick) - KEYBOARD_WIDTH
        const ny = yFromPitch(note.pitch)
        const nw = note.durationTicks * pixelsPerTick
        if (nx < xHi && nx + nw > xLo && ny < yHi && ny + noteHeight > yLo) {
          hit.add(note.index)
        }
      }
      setSelectedNotes(prev => {
        if (!additive) return hit
        const next = new Set(prev)
        for (const i of hit) next.add(i)
        return next
      })
      return
    }

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
        durationTicks: Math.max(snap, note.durationTicks),
      })
      useProjectStore.getState().markDirty()
      await refreshNotes()
    } catch (err) { console.warn('update_midi_note failed', err) }
  }, [notes, activeTrackId, activeClipId, refreshNotes, snap])

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
    if ((e.ctrlKey || e.metaKey) && e.shiftKey) {
      e.preventDefault()
      setNoteHeight(prev => Math.max(MIN_NOTE_HEIGHT, Math.min(MAX_NOTE_HEIGHT, prev * (e.deltaY < 0 ? 1.15 : 0.87))))
    } else if (e.ctrlKey || e.metaKey) {
      e.preventDefault()
      setPixelsPerTick(prev => Math.max(0.02, Math.min(1, prev * (e.deltaY < 0 ? 1.15 : 0.87))))
    } else if (e.shiftKey) {
      setScrollX(prev => Math.max(0, prev + e.deltaY))
    } else {
      setScrollY(prev => Math.max(0, Math.min(totalHeight - 200, prev + e.deltaY)))
    }
  }, [totalHeight])

  const updateNotes = useCallback(async (
    indices: number[],
    patch: (n: Note) => Partial<Pick<Note, 'pitch' | 'startTick' | 'durationTicks' | 'velocity'>>,
  ) => {
    if (!activeTrackId || !activeClipId || indices.length === 0) return
    try {
      for (const idx of indices) {
        const note = notes.find(n => n.index === idx)
        if (!note) continue
        const args: Record<string, unknown> = {
          trackId: activeTrackId,
          clipId: activeClipId,
          noteIndex: idx,
          ...patch(note),
        }
        await invoke('update_midi_note', args)
      }
      useProjectStore.getState().markDirty()
      await refreshNotes()
    } catch (err) { console.warn('update selection failed', err) }
  }, [activeTrackId, activeClipId, notes, refreshNotes])

  const [toolsOpen, setToolsOpen] = useState(false)
  const [qOpen, setQOpen] = useState(false)
  const [qStrength, setQStrength] = useState(100)
  const [qMode, setQMode] = useState<'start' | 'end' | 'both'>('start')
  const [swingPct, setSwingPct] = useState(0)

  const runTransform = useCallback(async (
    kind: 'legato' | 'staccato' | 'humanizeTime' | 'humanizeVel' | 'humanizeLen' | 'flip' | 'reverse' | 'crescendo' | 'decrescendo' | 'velFull' | 'velDouble' | 'velHalf' | 'velReset',
  ) => {
    if (!activeTrackId || !activeClipId) return
    setToolsOpen(false)
    const sel = selectedNotes.size > 0
      ? notes.filter(n => selectedNotes.has(n.index))
      : notes
    if (sel.length === 0) return
    const sorted = [...sel].sort((a, b) => a.startTick - b.startTick || a.pitch - b.pitch)

    const patches = new Map<number, Partial<Pick<Note, 'pitch' | 'startTick' | 'durationTicks' | 'velocity'>>>()

    if (kind === 'legato') {
      for (let i = 0; i < sorted.length; i++) {
        const cur = sorted[i]
        const next = sorted.slice(i + 1).find(n => n.startTick > cur.startTick)
        if (!next) continue
        const newDur = Math.max(snap, next.startTick - cur.startTick)
        if (newDur !== cur.durationTicks) patches.set(cur.index, { durationTicks: newDur })
      }
    } else if (kind === 'staccato') {
      for (const n of sorted) {
        const newDur = Math.max(snap, Math.round(n.durationTicks * 0.5))
        patches.set(n.index, { durationTicks: newDur })
      }
    } else if (kind === 'humanizeTime') {
      for (const n of sorted) {
        const jitter = Math.round((Math.random() - 0.5) * (snap * 0.25))
        patches.set(n.index, { startTick: Math.max(0, n.startTick + jitter) })
      }
    } else if (kind === 'humanizeVel') {
      for (const n of sorted) {
        const jitter = (Math.random() - 0.5) * 0.2
        patches.set(n.index, { velocity: Math.max(0.05, Math.min(1, n.velocity + jitter)) })
      }
    } else if (kind === 'humanizeLen') {
      for (const n of sorted) {
        const jitter = Math.round((Math.random() - 0.5) * (snap * 0.25))
        patches.set(n.index, { durationTicks: Math.max(snap, n.durationTicks + jitter) })
      }
    } else if (kind === 'flip') {
      const pitches = sorted.map(n => n.pitch)
      const lo = Math.min(...pitches)
      const hi = Math.max(...pitches)
      const center = (lo + hi) / 2
      for (const n of sorted) {
        const newPitch = Math.max(0, Math.min(127, Math.round(2 * center - n.pitch)))
        if (newPitch !== n.pitch) patches.set(n.index, { pitch: newPitch })
      }
    } else if (kind === 'reverse') {
      const lo = sorted[0].startTick
      const hi = Math.max(...sorted.map(n => n.startTick + n.durationTicks))
      for (const n of sorted) {
        const newStart = Math.max(0, lo + (hi - (n.startTick + n.durationTicks)))
        if (newStart !== n.startTick) patches.set(n.index, { startTick: newStart })
      }
    } else if (kind === 'velFull') {
      for (const n of sorted) patches.set(n.index, { velocity: 1 })
    } else if (kind === 'velDouble') {
      for (const n of sorted) patches.set(n.index, { velocity: Math.min(1, n.velocity * 1.5) })
    } else if (kind === 'velHalf') {
      for (const n of sorted) patches.set(n.index, { velocity: Math.max(0.05, n.velocity * 0.6) })
    } else if (kind === 'velReset') {
      for (const n of sorted) patches.set(n.index, { velocity: 0.8 })
    } else if (kind === 'crescendo' || kind === 'decrescendo') {
      const startVel = kind === 'crescendo' ? 0.3 : 1.0
      const endVel = kind === 'crescendo' ? 1.0 : 0.3
      const span = Math.max(1, sorted.length - 1)
      for (let i = 0; i < sorted.length; i++) {
        const t = i / span
        const v = startVel + (endVel - startVel) * t
        patches.set(sorted[i].index, { velocity: Math.max(0.05, Math.min(1, v)) })
      }
    }

    if (patches.size === 0) return
    try {
      for (const [idx, patch] of patches) {
        await invoke('update_midi_note', {
          trackId: activeTrackId,
          clipId: activeClipId,
          noteIndex: idx,
          ...patch,
        })
      }
      useProjectStore.getState().markDirty()
      await refreshNotes()
    } catch (err) { console.warn('transform failed', err) }
  }, [activeTrackId, activeClipId, notes, selectedNotes, snap, refreshNotes])

  const runQuantize = useCallback(async () => {
    if (!activeTrackId || !activeClipId) return
    setQOpen(false)
    const sel = selectedNotes.size > 0
      ? notes.filter(n => selectedNotes.has(n.index))
      : notes
    if (sel.length === 0) return
    const strength = qStrength / 100
    const swing = swingPct / 100
    try {
      for (const n of sel) {
        const snappedStart = Math.round(n.startTick / snap) * snap
        let targetStart = snappedStart
        if (swing !== 0) {
          const beatIndex = Math.round(n.startTick / snap)
          if (beatIndex % 2 === 1) {
            targetStart = snappedStart + Math.round(snap * swing * 0.5)
          }
        }
        const newStart = Math.round(n.startTick + (targetStart - n.startTick) * strength)

        const end = n.startTick + n.durationTicks
        const snappedEnd = Math.round(end / snap) * snap
        const newEnd = Math.round(end + (snappedEnd - end) * strength)

        const patch: Partial<Pick<Note, 'startTick' | 'durationTicks'>> = {}
        if (qMode === 'start' || qMode === 'both') patch.startTick = Math.max(0, newStart)
        if (qMode === 'end' || qMode === 'both') {
          const anchor = patch.startTick ?? n.startTick
          patch.durationTicks = Math.max(snap, newEnd - anchor)
        }
        if (Object.keys(patch).length === 0) continue
        await invoke('update_midi_note', {
          trackId: activeTrackId, clipId: activeClipId, noteIndex: n.index, ...patch,
        })
      }
      useProjectStore.getState().markDirty()
      await refreshNotes()
    } catch (err) { console.warn('quantize failed', err) }
  }, [activeTrackId, activeClipId, notes, selectedNotes, snap, qStrength, qMode, swingPct, refreshNotes])

  useEffect(() => {
    if (!qOpen) return
    const close = (e: MouseEvent) => {
      const t = e.target as HTMLElement
      if (t.closest('[data-q-popover]')) return
      setQOpen(false)
    }
    window.addEventListener('click', close)
    return () => window.removeEventListener('click', close)
  }, [qOpen])

  const insertNotesInPlace = useCallback(async () => {
    if (!activeTrackId || !activeClipId) return
    const clip = clipboardRef.current
    if (clip.length === 0) return
    const newIndices: number[] = []
    try {
      for (const n of clip) {
        const newIndex = await invoke<number>('add_midi_note', {
          trackId: activeTrackId,
          clipId: activeClipId,
          pitch: n.pitch,
          startTick: Math.max(0, n.startTick),
          durationTicks: n.durationTicks,
          velocity: n.velocity,
        })
        newIndices.push(newIndex)
      }
      useProjectStore.getState().markDirty()
      await refreshNotes()
      setSelectedNotes(new Set(newIndices))
    } catch (err) { console.warn('paste-in-place failed', err) }
  }, [activeTrackId, activeClipId, refreshNotes])

  const insertNotesFromClipboard = useCallback(async (originTick: number) => {
    if (!activeTrackId || !activeClipId) return
    const clip = clipboardRef.current
    if (clip.length === 0) return
    const earliest = clip.reduce((m, n) => Math.min(m, n.startTick), Infinity)
    const newIndices: number[] = []
    try {
      for (const n of clip) {
        const offset = n.startTick - earliest
        const newIndex = await invoke<number>('add_midi_note', {
          trackId: activeTrackId,
          clipId: activeClipId,
          pitch: n.pitch,
          startTick: Math.max(0, originTick + offset),
          durationTicks: n.durationTicks,
          velocity: n.velocity,
        })
        newIndices.push(newIndex)
      }
      useProjectStore.getState().markDirty()
      await refreshNotes()
      setSelectedNotes(new Set(newIndices))
    } catch (err) { console.warn('paste failed', err) }
  }, [activeTrackId, activeClipId, refreshNotes])

  const handleKeyDown = useCallback(async (e: KeyboardEvent) => {
    if (!activeTrackId || !activeClipId) return
    if (!focusedRef.current) return
    const target = e.target as HTMLElement
    if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) return

    const consume = () => {
      e.preventDefault()
      e.stopImmediatePropagation()
    }

    const ctrl = e.ctrlKey || e.metaKey

    if (ctrl && e.key.toLowerCase() === 'a') {
      consume()
      setSelectedNotes(new Set(notes.map(n => n.index)))
      return
    }

    if (ctrl && e.key.toLowerCase() === 'c') {
      if (selectedNotes.size === 0) return
      consume()
      clipboardRef.current = notes.filter(n => selectedNotes.has(n.index)).map(n => ({ ...n }))
      return
    }

    if (ctrl && e.key.toLowerCase() === 'x') {
      if (selectedNotes.size === 0) return
      consume()
      clipboardRef.current = notes.filter(n => selectedNotes.has(n.index)).map(n => ({ ...n }))
      const indicesDesc = Array.from(selectedNotes).sort((a, b) => b - a)
      try {
        for (const idx of indicesDesc) {
          await invoke('delete_midi_note', { trackId: activeTrackId, clipId: activeClipId, noteIndex: idx })
        }
        useProjectStore.getState().markDirty()
        setSelectedNotes(new Set())
        await refreshNotes()
      } catch (err) { console.warn('cut failed', err) }
      return
    }

    if (ctrl && e.shiftKey && e.key.toLowerCase() === 'v') {
      consume()
      await insertNotesInPlace()
      return
    }

    if (ctrl && e.key.toLowerCase() === 'v') {
      consume()
      const ts = useTransportStore.getState()
      let originTick: number
      if (ts.editCursorTicks != null) {
        originTick = ts.editCursorTicks
      } else {
        const sr = ts.sampleRate || 48000
        const samplesPerTick = (sr * 60) / (ts.bpm * PPQ)
        originTick = Math.round(ts.positionSamples / samplesPerTick)
      }
      await insertNotesFromClipboard(originTick)
      return
    }

    if (ctrl && e.altKey && e.key.toLowerCase() === 'a') {
      if (selectedNotes.size < 2) return
      consume()
      const sel = notes.filter(n => selectedNotes.has(n.index))
      const sorted = [...sel].sort((a, b) => a.pitch - b.pitch)
      const base = Math.min(...sel.map(n => n.startTick))
      const orderByIndex = new Map(sorted.map((n, i) => [n.index, i]))
      await updateNotes(Array.from(selectedNotes), (n) => {
        const i = orderByIndex.get(n.index) ?? 0
        return { startTick: base + i * snap }
      })
      return
    }

    if (ctrl && e.key.toLowerCase() === 'd') {
      if (selectedNotes.size === 0) return
      consume()
      const sel = notes.filter(n => selectedNotes.has(n.index))
      const earliest = Math.min(...sel.map(n => n.startTick))
      const latestEnd = Math.max(...sel.map(n => n.startTick + n.durationTicks))
      const span = latestEnd - earliest
      clipboardRef.current = sel.map(n => ({ ...n }))
      await insertNotesFromClipboard(earliest + span)
      return
    }

    if (ctrl && e.key.toLowerCase() === 'q') {
      if (selectedNotes.size === 0) return
      consume()
      const indices = Array.from(selectedNotes)
      await updateNotes(indices, n => ({
        startTick: Math.round(n.startTick / snap) * snap,
      }))
      return
    }

    if (selectedNotes.size === 0) return

    if (e.key === 'Delete' || e.key === 'Backspace') {
      consume()
      const indicesDesc = Array.from(selectedNotes).sort((a, b) => b - a)
      try {
        for (const idx of indicesDesc) {
          await invoke('delete_midi_note', { trackId: activeTrackId, clipId: activeClipId, noteIndex: idx })
        }
        useProjectStore.getState().markDirty()
        setSelectedNotes(new Set())
        await refreshNotes()
      } catch (err) { console.warn('delete selected failed', err) }
      return
    }

    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      consume()
      const step = ctrl ? 12 : 1
      const dir = e.key === 'ArrowUp' ? 1 : -1
      const indices = Array.from(selectedNotes)
      await updateNotes(indices, n => ({ pitch: Math.max(0, Math.min(127, n.pitch + step * dir)) }))
      return
    }

    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      consume()
      const dir = e.key === 'ArrowRight' ? 1 : -1
      const indices = Array.from(selectedNotes)
      await updateNotes(indices, n => ({ startTick: Math.max(0, n.startTick + snap * dir) }))
      return
    }
  }, [activeTrackId, activeClipId, selectedNotes, refreshNotes, notes, snap, updateNotes, insertNotesFromClipboard, insertNotesInPlace])

  useEffect(() => {
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [handleKeyDown])

  useEffect(() => {
    if (!toolsOpen) return
    const close = () => setToolsOpen(false)
    window.addEventListener('click', close)
    return () => window.removeEventListener('click', close)
  }, [toolsOpen])

  const zoomToFit = useCallback(() => {
    const subject = selectedNotes.size > 0
      ? notes.filter(n => selectedNotes.has(n.index))
      : notes
    if (subject.length === 0) return
    const container = containerRef.current
    if (!container) return
    const rect = container.getBoundingClientRect()
    const w = rect.width - KEYBOARD_WIDTH
    const h = rect.height - RULER_HEIGHT - VELOCITY_LANE_HEIGHT

    const tLo = Math.min(...subject.map(n => n.startTick))
    const tHi = Math.max(...subject.map(n => n.startTick + n.durationTicks))
    const pLo = Math.min(...subject.map(n => n.pitch))
    const pHi = Math.max(...subject.map(n => n.pitch))

    const margin = 80
    const tickSpan = Math.max(1, tHi - tLo)
    const newPpt = Math.max(0.02, Math.min(1, (w - margin) / tickSpan))

    const pitchSpan = Math.max(1, pHi - pLo + 2)
    const newNh = Math.max(MIN_NOTE_HEIGHT, Math.min(MAX_NOTE_HEIGHT, h / pitchSpan))

    setPixelsPerTick(newPpt)
    setNoteHeight(newNh)
    setScrollX(Math.max(0, tLo * newPpt - 20))
    const centerPitch = (pLo + pHi) / 2
    setScrollY(Math.max(0, (TOTAL_NOTES - 1 - centerPitch) * newNh - h / 2))
  }, [notes, selectedNotes])

  const transportPosition = useTransportStore(s => s.positionSamples)
  const transportBpm = useTransportStore(s => s.bpm)
  const transportSr = useTransportStore(s => s.sampleRate)

  useEffect(() => {
    if (!followPlayhead) return
    const sr = transportSr || 48000
    const samplesPerTick = (sr * 60) / (transportBpm * PPQ)
    const playheadTick = transportPosition / samplesPerTick
    const container = containerRef.current
    if (!container) return
    const w = container.getBoundingClientRect().width - KEYBOARD_WIDTH
    const x = playheadTick * pixelsPerTick - scrollX
    if (x < 40 || x > w - 40) {
      setScrollX(Math.max(0, playheadTick * pixelsPerTick - w / 3))
    }
  }, [followPlayhead, pixelsPerTick, scrollX, transportPosition, transportBpm, transportSr])

  const emptyHint = !activeTrackId || !activeClipId
    ? 'Select a MIDI track or double-click a MIDI clip to edit notes.'
    : null

  return (
    <div ref={containerRef}
      onMouseEnter={() => { focusedRef.current = true }}
      onMouseLeave={() => { focusedRef.current = false }}
      style={{
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
        {selectedNotes.size > 0 && (
          <span style={{
            fontSize: 9, fontWeight: 600, color: hw.accent,
            background: hw.accentDim, padding: '1px 6px',
            borderRadius: hw.radius.sm,
          }}>
            {selectedNotes.size} selected
          </span>
        )}
        <div style={{ flex: 1 }} />

        <div style={{ display: 'flex', gap: 1 }}>
          {(['draw', 'select', 'erase', 'chord'] as const).map(t => (
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

        {tool === 'chord' && (
          <>
            <select
              value={chordType}
              onChange={e => setChordType(e.target.value as keyof typeof CHORD_TYPES)}
              title="Chord type"
              style={{
                fontSize: 9, background: 'rgba(255,255,255,0.04)', color: hw.accent,
                border: `1px solid ${hw.accentGlow}`, borderRadius: hw.radius.sm, padding: '1px 4px',
              }}
            >
              {Object.entries(CHORD_TYPES).map(([k, v]) => (
                <option key={k} value={k}>{v.name}</option>
              ))}
            </select>
            <select
              value={chordInversion}
              onChange={e => setChordInversion(Number(e.target.value))}
              title="Chord inversion"
              style={{
                fontSize: 9, background: 'rgba(255,255,255,0.04)', color: hw.textMuted,
                border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, padding: '1px 4px',
              }}
            >
              <option value={0}>Root</option>
              <option value={1}>1st inv</option>
              <option value={2}>2nd inv</option>
              <option value={3}>3rd inv</option>
            </select>
            {chordType === 'custom' && (
              <div style={{ display: 'flex', gap: 1, padding: '0 2px' }} title="Select intervals (semitones) to stamp">
                {INTERVAL_LABELS.map((label, iv) => {
                  const on = customChordSet.has(iv)
                  return (
                    <button
                      key={iv}
                      onClick={() => setCustomChordSet(prev => {
                        const next = new Set(prev)
                        if (next.has(iv)) next.delete(iv); else next.add(iv)
                        if (next.size === 0) next.add(0)
                        return next
                      })}
                      title={`${label} (${iv} st)`}
                      style={{
                        width: 16, height: 14, fontSize: 8, fontWeight: 600,
                        color: on ? hw.textBright : hw.textFaint,
                        background: on ? hw.accentDim : 'rgba(255,255,255,0.04)',
                        border: `1px solid ${on ? hw.accentGlow : hw.border}`,
                        borderRadius: 2, padding: 0, cursor: 'pointer',
                      }}
                    >
                      {label}
                    </button>
                  )
                })}
              </div>
            )}
          </>
        )}

        <select
          value={scaleRoot}
          onChange={e => setScaleRoot(Number(e.target.value))}
          title="Scale root note"
          style={{
            fontSize: 9, background: 'rgba(255,255,255,0.04)', color: hw.textMuted,
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, padding: '1px 4px',
          }}
        >
          {NOTE_NAMES_SHARP.map((n, i) => (
            <option key={i} value={i}>{n}</option>
          ))}
        </select>
        <select
          value={scaleType}
          onChange={e => setScaleType(e.target.value as keyof typeof SCALE_TYPES)}
          title="Scale type"
          style={{
            fontSize: 9, background: 'rgba(255,255,255,0.04)',
            color: scaleType === 'chromatic' ? hw.textFaint : hw.accent,
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, padding: '1px 4px',
          }}
        >
          {Object.entries(SCALE_TYPES).map(([k, v]) => (
            <option key={k} value={k}>{v.name}</option>
          ))}
        </select>
        <button
          onClick={() => setSnapToScale(v => !v)}
          title="Snap new notes to scale"
          disabled={scaleType === 'chromatic'}
          style={{
            padding: '1px 5px', fontSize: 9, fontWeight: 600,
            color: snapToScale ? hw.accent : hw.textFaint,
            background: snapToScale ? hw.accentDim : 'transparent',
            border: `1px solid ${snapToScale ? hw.accentGlow : hw.border}`,
            borderRadius: hw.radius.sm,
            opacity: scaleType === 'chromatic' ? 0.4 : 1,
          }}
        >
          SNAP
        </button>

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

        <button
          onClick={zoomToFit}
          title={selectedNotes.size > 0 ? 'Zoom to selection' : 'Zoom to fit all notes'}
          style={{
            padding: '1px 6px', fontSize: 9, fontWeight: 600,
            color: hw.textMuted,
            background: 'rgba(255,255,255,0.04)',
            border: `1px solid ${hw.border}`,
            borderRadius: hw.radius.sm,
          }}
        >
          FIT
        </button>
        <button
          onClick={() => setFollowPlayhead(v => !v)}
          title="Follow playhead"
          style={{
            padding: '1px 6px', fontSize: 9, fontWeight: 600,
            color: followPlayhead ? hw.accent : hw.textFaint,
            background: followPlayhead ? hw.accentDim : 'rgba(255,255,255,0.04)',
            border: `1px solid ${followPlayhead ? hw.accentGlow : hw.border}`,
            borderRadius: hw.radius.sm,
          }}
        >
          FOLLOW
        </button>
        <button
          onClick={() => setShowMinimap(v => !v)}
          title="Toggle minimap overview"
          style={{
            padding: '1px 6px', fontSize: 9, fontWeight: 600,
            color: showMinimap ? hw.accent : hw.textFaint,
            background: showMinimap ? hw.accentDim : 'rgba(255,255,255,0.04)',
            border: `1px solid ${showMinimap ? hw.accentGlow : hw.border}`,
            borderRadius: hw.radius.sm,
          }}
        >
          MAP
        </button>
        <button
          onClick={() => setGhostMode(m => m === 'off' ? 'track' : m === 'track' ? 'all' : 'off')}
          title={
            ghostMode === 'off' ? 'Ghost notes: OFF (click to show same-track)' :
            ghostMode === 'track' ? 'Ghost notes: same track (click to show all)' :
            'Ghost notes: all tracks (click to hide)'
          }
          style={{
            padding: '1px 6px', fontSize: 9, fontWeight: 600,
            color: ghostMode !== 'off' ? hw.accent : hw.textFaint,
            background: ghostMode !== 'off' ? hw.accentDim : 'rgba(255,255,255,0.04)',
            border: `1px solid ${ghostMode !== 'off' ? hw.accentGlow : hw.border}`,
            borderRadius: hw.radius.sm,
          }}
        >
          GHOST{ghostMode === 'track' ? ':T' : ghostMode === 'all' ? ':A' : ''}
        </button>
        <button
          onClick={() => setVelFromY(v => !v)}
          title="Velocity from vertical click position inside the note row"
          style={{
            padding: '1px 6px', fontSize: 9, fontWeight: 600,
            color: velFromY ? hw.accent : hw.textFaint,
            background: velFromY ? hw.accentDim : 'rgba(255,255,255,0.04)',
            border: `1px solid ${velFromY ? hw.accentGlow : hw.border}`,
            borderRadius: hw.radius.sm,
          }}
        >
          VEL-Y
        </button>

        <div style={{ position: 'relative' }}>
          <button
            onClick={(e) => { e.stopPropagation(); setQOpen(v => !v) }}
            title="Quantize options (Ctrl+Q applies)"
            style={{
              padding: '1px 8px', fontSize: 9, fontWeight: 600,
              color: qOpen ? hw.accent : hw.textMuted,
              background: qOpen ? hw.accentDim : 'rgba(255,255,255,0.04)',
              border: `1px solid ${qOpen ? hw.accentGlow : hw.border}`,
              borderRadius: hw.radius.sm, textTransform: 'uppercase',
            }}
          >
            Q ▾
          </button>
          {qOpen && (
            <div
              data-q-popover
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => e.stopPropagation()}
              style={{
                position: 'absolute', top: 22, right: 0, zIndex: 500,
                minWidth: 220, padding: 10,
                background: 'rgba(12,12,18,0.96)',
                border: `1px solid ${hw.borderLight}`,
                borderRadius: hw.radius.md,
                boxShadow: '0 8px 32px rgba(0,0,0,0.55)',
                backdropFilter: hw.blur.md,
                display: 'flex', flexDirection: 'column', gap: 8,
              }}
            >
              <div style={{ fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>Quantize</div>
              <div>
                <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: hw.textMuted }}>
                  <span>Strength</span>
                  <span style={{ color: hw.textPrimary, fontWeight: 600 }}>{qStrength}%</span>
                </div>
                <input type="range" min={0} max={100} step={1} value={qStrength}
                  onChange={(e) => setQStrength(Number(e.target.value))}
                  style={{ width: '100%', accentColor: hw.accent }} />
              </div>
              <div>
                <div style={{ fontSize: 9, color: hw.textMuted, marginBottom: 2 }}>Apply to</div>
                <div style={{ display: 'flex', gap: 2 }}>
                  {(['start','end','both'] as const).map(m => (
                    <button key={m} onClick={() => setQMode(m)} style={{
                      flex: 1, padding: '3px 0', fontSize: 9, fontWeight: 600,
                      color: qMode === m ? hw.accent : hw.textFaint,
                      background: qMode === m ? hw.accentDim : 'transparent',
                      border: `1px solid ${qMode === m ? hw.accentGlow : hw.border}`,
                      borderRadius: hw.radius.sm, textTransform: 'uppercase',
                    }}>
                      {m}
                    </button>
                  ))}
                </div>
              </div>
              <div>
                <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: hw.textMuted }}>
                  <span>Swing</span>
                  <span style={{ color: hw.textPrimary, fontWeight: 600 }}>{swingPct}%</span>
                </div>
                <input type="range" min={0} max={100} step={1} value={swingPct}
                  onChange={(e) => setSwingPct(Number(e.target.value))}
                  style={{ width: '100%', accentColor: hw.accent }} />
              </div>
              <button
                onClick={runQuantize}
                style={{
                  padding: '5px 10px', fontSize: 10, fontWeight: 700,
                  color: '#fff', background: hw.accent,
                  border: 'none', borderRadius: hw.radius.sm, cursor: 'pointer',
                }}
              >
                Apply quantize
              </button>
            </div>
          )}
        </div>

        <div style={{ position: 'relative' }}>
          <button
            onClick={() => setToolsOpen(v => !v)}
            title="Note tools: legato, humanize, flip, reverse…"
            style={{
              padding: '1px 8px', fontSize: 9, fontWeight: 600,
              color: toolsOpen ? hw.accent : hw.textMuted,
              background: toolsOpen ? hw.accentDim : 'rgba(255,255,255,0.04)',
              border: `1px solid ${toolsOpen ? hw.accentGlow : hw.border}`,
              borderRadius: hw.radius.sm, textTransform: 'uppercase',
            }}
          >
            Tools ▾
          </button>
          {toolsOpen && (
            <div
              onMouseDown={(e) => e.stopPropagation()}
              style={{
                position: 'absolute', top: 22, right: 0, zIndex: 500,
                minWidth: 200, padding: 4,
                background: 'rgba(12,12,18,0.96)',
                border: `1px solid ${hw.borderLight}`,
                borderRadius: hw.radius.md,
                boxShadow: '0 8px 32px rgba(0,0,0,0.55)',
                backdropFilter: hw.blur.md,
              }}
            >
              {[
                ['Legato', 'legato' as const, 'Extend notes to next'],
                ['Staccato', 'staccato' as const, 'Halve durations'],
                [null, null, null],
                ['Humanize timing', 'humanizeTime' as const, 'Jitter start ±25%'],
                ['Humanize velocity', 'humanizeVel' as const, 'Jitter velocity ±0.1'],
                ['Humanize length', 'humanizeLen' as const, 'Jitter duration ±25%'],
                [null, null, null],
                ['Flip vertical', 'flip' as const, 'Invert pitch around center'],
                ['Reverse', 'reverse' as const, 'Mirror notes in time'],
                [null, null, null],
                ['Crescendo', 'crescendo' as const, 'Ramp velocity up'],
                ['Decrescendo', 'decrescendo' as const, 'Ramp velocity down'],
                [null, null, null],
                ['Set velocity full', 'velFull' as const, 'velocity = 1.0'],
                ['Scale velocity ×1.5', 'velDouble' as const, 'Boost velocity'],
                ['Scale velocity ×0.6', 'velHalf' as const, 'Reduce velocity'],
                ['Reset velocity', 'velReset' as const, 'velocity = 0.8'],
              ].map(([label, key, hint], i) => {
                if (label === null) {
                  return <div key={`s${i}`} style={{ height: 1, background: hw.border, margin: '3px 0' }} />
                }
                return (
                  <ToolMenuItem
                    key={key as string}
                    label={label as string}
                    hint={hint as string}
                    onClick={() => runTransform(key as Parameters<typeof runTransform>[0])}
                  />
                )
              })}
            </div>
          )}
        </div>

        <DetachButton panelId="pianoRoll" />
      </div>

      <div style={{ flex: 1, display: 'flex', overflow: 'hidden', position: 'relative' }}>
        <PianoKeyboard
          width={KEYBOARD_WIDTH}
          noteHeight={noteHeight}
          scrollY={scrollY}
          totalNotes={TOTAL_NOTES}
          scaleRoot={scaleRoot}
          scaleIntervals={SCALE_TYPES[scaleType].intervals}
          showScale={scaleType !== 'chromatic'}
        />

        <div style={{ flex: 1, position: 'relative', overflow: 'hidden' }}>
          <canvas
            ref={canvasRef}
            onMouseDown={handleMouseDown}
            onMouseMove={handleMouseMove}
            onMouseUp={handleMouseUp}
            onMouseLeave={(e) => { handleMouseUp(); setHoverInfo(null); void e }}
            onWheel={handleWheel}
            onContextMenu={(e) => {
              e.preventDefault()
              const rect = canvasRef.current!.getBoundingClientRect()
              const mx = e.clientX - rect.left
              const my = e.clientY - rect.top
              for (const n of notes) {
                const nx = xFromTick(n.startTick) - KEYBOARD_WIDTH
                const ny = yFromPitch(n.pitch)
                const nw = n.durationTicks * pixelsPerTick
                if (mx >= nx && mx <= nx + nw && my >= ny && my <= ny + noteHeight) {
                  setNoteCtx({ x: e.clientX, y: e.clientY, noteIndex: n.index })
                  return
                }
              }
              setNoteCtx(null)
            }}
            style={{ display: 'block', cursor: tool === 'draw' || tool === 'chord' ? 'crosshair' : tool === 'erase' ? 'not-allowed' : 'default' }}
          />
          {hoverInfo && (
            <div style={{
              position: 'absolute',
              left: Math.min(hoverInfo.x + 12, 9999),
              top: Math.max(hoverInfo.y - 22, 4),
              pointerEvents: 'none',
              padding: '3px 6px',
              background: 'rgba(12,12,18,0.95)',
              border: `1px solid ${hw.borderLight}`,
              borderRadius: hw.radius.sm,
              color: hw.textBright,
              fontSize: 10,
              fontFamily: 'ui-monospace, monospace',
              whiteSpace: 'nowrap',
              zIndex: 10,
            }}>
              {hoverInfo.label}
            </div>
          )}
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

      {showMinimap && (
        <Minimap
          notes={notes}
          height={MINIMAP_HEIGHT}
          keyboardWidth={KEYBOARD_WIDTH}
          scrollX={scrollX}
          viewWidth={viewportWidth}
          totalTicks={Math.max(PPQ * 32, notes.reduce((m, n) => Math.max(m, n.startTick + n.durationTicks), 0) + PPQ * 4)}
          pixelsPerTick={pixelsPerTick}
          onScroll={(sx) => setScrollX(Math.max(0, sx))}
        />
      )}
      {noteCtx && (() => {
        const note = notes.find(n => n.index === noteCtx.noteIndex)
        if (!note) return null
        const close = () => setNoteCtx(null)
        const isInSel = selectedNotes.has(note.index)
        const targetIndices = isInSel && selectedNotes.size > 1 ? Array.from(selectedNotes) : [note.index]
        const setVel = async (v: number) => {
          close()
          await updateNotes(targetIndices, () => ({ velocity: v }))
        }
        const toggleMute = async () => {
          close()
          const next = !note.muted
          if (!activeTrackId || !activeClipId) return
          try {
            for (const idx of targetIndices) {
              await invoke('update_midi_note', {
                trackId: activeTrackId, clipId: activeClipId,
                noteIndex: idx, muted: next,
              })
            }
            useProjectStore.getState().markDirty()
            await refreshNotes()
          } catch (err) { console.warn('toggle mute failed', err) }
        }
        const duplicate = async () => {
          close()
          if (!activeTrackId || !activeClipId) return
          try {
            const newIndices: number[] = []
            for (const idx of targetIndices) {
              const n = notes.find(x => x.index === idx)
              if (!n) continue
              const i = await invoke<number>('add_midi_note', {
                trackId: activeTrackId, clipId: activeClipId,
                pitch: n.pitch, startTick: n.startTick + n.durationTicks,
                durationTicks: n.durationTicks, velocity: n.velocity,
              })
              newIndices.push(i)
            }
            useProjectStore.getState().markDirty()
            await refreshNotes()
            setSelectedNotes(new Set(newIndices))
          } catch (err) { console.warn('duplicate failed', err) }
        }
        const quantize = async () => {
          close()
          await updateNotes(targetIndices, (n) => ({
            startTick: Math.round(n.startTick / snap) * snap,
          }))
        }
        const del = async () => {
          close()
          if (!activeTrackId || !activeClipId) return
          const desc = targetIndices.slice().sort((a, b) => b - a)
          try {
            for (const idx of desc) {
              await invoke('delete_midi_note', {
                trackId: activeTrackId, clipId: activeClipId, noteIndex: idx,
              })
            }
            useProjectStore.getState().markDirty()
            setSelectedNotes(new Set())
            await refreshNotes()
          } catch (err) { console.warn('delete note failed', err) }
        }
        const label = targetIndices.length > 1 ? `${targetIndices.length} notes` : noteName(note.pitch)
        return (
          <>
            <div
              onMouseDown={close}
              onContextMenu={(e) => { e.preventDefault(); close() }}
              style={{ position: 'fixed', inset: 0, zIndex: 9999 }}
            />
            <div
              onMouseDown={(e) => e.stopPropagation()}
              style={{
                position: 'fixed', left: noteCtx.x, top: noteCtx.y, zIndex: 10000,
                minWidth: 180, padding: 4,
                background: 'rgba(12,12,18,0.96)',
                border: `1px solid ${hw.borderLight}`,
                borderRadius: hw.radius.md,
                boxShadow: '0 8px 32px rgba(0,0,0,0.55)',
                backdropFilter: hw.blur.md,
              }}
            >
              <div style={{ padding: '4px 8px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
                {label}
              </div>
              <NoteMenuItem label={note.muted ? 'Unmute' : 'Mute'} onClick={toggleMute} />
              <NoteMenuItem label="Duplicate" shortcut="Ctrl+D" onClick={duplicate} />
              <NoteMenuItem label="Quantize" shortcut="Ctrl+Q" onClick={quantize} />
              <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
              <div style={{ padding: '4px 8px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
                Velocity
              </div>
              <div style={{ display: 'flex', gap: 2, padding: '0 4px 2px' }}>
                {[0.25, 0.5, 0.75, 1.0].map(v => (
                  <button key={v} onClick={() => setVel(v)}
                    style={{
                      flex: 1, height: 20, fontSize: 9, color: hw.textSecondary,
                      background: 'rgba(255,255,255,0.04)', border: `1px solid ${hw.border}`,
                      borderRadius: hw.radius.sm, cursor: 'pointer', padding: 0,
                    }}
                  >{Math.round(v * 100)}</button>
                ))}
              </div>
              <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
              <NoteMenuItem label="Delete" shortcut="Del" danger onClick={del} />
            </div>
          </>
        )
      })()}
    </div>
  )
}

function NoteMenuItem({ label, shortcut, danger, onClick }: {
  label: string; shortcut?: string; danger?: boolean; onClick: () => void
}) {
  return (
    <button
      onClick={onClick}
      style={{
        width: '100%', display: 'flex', alignItems: 'center',
        padding: '5px 8px', gap: 8, border: 'none',
        background: 'transparent', color: danger ? hw.red : hw.textSecondary,
        fontSize: 11, cursor: 'pointer', borderRadius: hw.radius.sm,
      }}
      onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.05)' }}
      onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent' }}
    >
      <span style={{ flex: 1, textAlign: 'left' }}>{label}</span>
      {shortcut && <span style={{ fontSize: 9, color: hw.textFaint }}>{shortcut}</span>}
    </button>
  )
}

function ToolMenuItem({ label, hint, onClick }: { label: string; hint: string; onClick: () => void }) {
  const [hover, setHover] = useState(false)
  return (
    <button
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onClick={onClick}
      style={{
        width: '100%', display: 'flex', flexDirection: 'column', alignItems: 'flex-start',
        padding: '4px 8px', border: 'none',
        background: hover ? hw.accentDim : 'transparent',
        color: hover ? hw.textBright : hw.textPrimary,
        borderRadius: hw.radius.sm, cursor: 'pointer', textAlign: 'left',
      }}
    >
      <span style={{ fontSize: 10, fontWeight: 600 }}>{label}</span>
      <span style={{ fontSize: 8, color: hw.textFaint, marginTop: 1 }}>{hint}</span>
    </button>
  )
}
