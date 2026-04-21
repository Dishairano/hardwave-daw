import { useCallback, useEffect, useRef, useState } from 'react'
import { hw } from '../../theme'
import {
  CC_LANE_RESOLUTION,
  CcLaneDefinition,
  laneDefaultValue,
  laneDisplayValue,
  useMidiCcStore,
} from '../../stores/midiCcStore'

export type CcTool = 'pencil' | 'line' | 'curve'

interface CcLaneProps {
  clipId: string
  def: CcLaneDefinition
  height: number
  keyboardWidth: number
  scrollX: number
  pixelsPerTick: number
  clipLengthTicks: number
  tool: CcTool
  onRemove: () => void
  onClear: () => void
  onChangeLane: (newId: string) => void
  allDefs: CcLaneDefinition[]
}

export function CcLane({
  clipId,
  def,
  height,
  keyboardWidth,
  scrollX,
  pixelsPerTick,
  clipLengthTicks,
  tool,
  onRemove,
  onClear,
  onChangeLane,
  allDefs,
}: CcLaneProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const [hover, setHover] = useState<{ slot: number; value: number } | null>(null)
  const lineAnchor = useRef<{ slot: number; value: number } | null>(null)
  const curveAnchors = useRef<Array<{ slot: number; value: number }>>([])

  const values = useMidiCcStore(s => s.values[clipId]?.[def.id])
  const setValueAt = useMidiCcStore(s => s.setValueAt)
  const setValuesRange = useMidiCcStore(s => s.setValuesRange)

  const laneWidthTicks = Math.max(1, clipLengthTicks)
  const slotToTick = useCallback((slot: number) => (slot / CC_LANE_RESOLUTION) * laneWidthTicks, [laneWidthTicks])
  const tickToSlot = useCallback(
    (tick: number) => Math.max(0, Math.min(CC_LANE_RESOLUTION - 1, Math.round((tick / laneWidthTicks) * CC_LANE_RESOLUTION))),
    [laneWidthTicks],
  )

  const draw = useCallback(() => {
    const canvas = canvasRef.current
    const container = containerRef.current
    if (!canvas || !container) return
    const w = container.clientWidth
    const h = height
    canvas.width = w * devicePixelRatio
    canvas.height = h * devicePixelRatio
    canvas.style.width = `${w}px`
    canvas.style.height = `${h}px`
    const ctx = canvas.getContext('2d')!
    ctx.setTransform(devicePixelRatio, 0, 0, devicePixelRatio, 0, 0)

    ctx.fillStyle = '#07070c'
    ctx.fillRect(0, 0, w, h)

    ctx.strokeStyle = 'rgba(255,255,255,0.04)'
    ctx.lineWidth = 1
    ctx.beginPath()
    ctx.moveTo(0, 0.5)
    ctx.lineTo(w, 0.5)
    ctx.stroke()

    const bipolar = def.kind === 'pitchBend'
    if (bipolar) {
      ctx.setLineDash([3, 3])
      ctx.strokeStyle = 'rgba(255,255,255,0.08)'
      ctx.beginPath()
      ctx.moveTo(0, h / 2)
      ctx.lineTo(w, h / 2)
      ctx.stroke()
      ctx.setLineDash([])
    } else {
      ctx.strokeStyle = 'rgba(255,255,255,0.03)'
      for (const pct of [0.25, 0.5, 0.75]) {
        const y = h - pct * h
        ctx.beginPath()
        ctx.moveTo(0, y)
        ctx.lineTo(w, y)
        ctx.stroke()
      }
    }

    ctx.fillStyle = hw.textFaint
    ctx.font = '8px Inter, ui-sans-serif, sans-serif'
    ctx.fillText(def.shortLabel, 4, 12)

    const slotPxWidth = (CC_LANE_RESOLUTION > 0 ? laneWidthTicks / CC_LANE_RESOLUTION : 1) * pixelsPerTick
    const defVal = laneDefaultValue(def)

    const valueAt = (i: number) => (values && i < values.length ? values[i] : defVal)

    for (let i = 0; i < CC_LANE_RESOLUTION; i++) {
      const v = valueAt(i)
      const centerTick = slotToTick(i) + laneWidthTicks / CC_LANE_RESOLUTION / 2
      const x = centerTick * pixelsPerTick - scrollX
      const bw = Math.max(1, slotPxWidth)
      if (x + bw < 0 || x - bw > w) continue

      if (bipolar) {
        const midY = h / 2
        const deflect = (v - 0.5) * 2
        const barH = Math.abs(deflect) * (h / 2 - 2)
        const y = deflect >= 0 ? midY - barH : midY
        const color = deflect >= 0 ? hw.accent : hw.secondary
        ctx.fillStyle = color
        ctx.globalAlpha = 0.65
        ctx.fillRect(x - bw / 2, y, bw, barH)
      } else {
        const barH = v * (h - 4)
        const y = h - barH
        ctx.fillStyle = hw.accent
        ctx.globalAlpha = 0.65
        ctx.fillRect(x - bw / 2, y, bw, barH)
      }
      ctx.globalAlpha = 1
    }

    if (hover) {
      const tick = slotToTick(hover.slot) + laneWidthTicks / CC_LANE_RESOLUTION / 2
      const x = tick * pixelsPerTick - scrollX
      ctx.strokeStyle = 'rgba(255,255,255,0.3)'
      ctx.setLineDash([2, 2])
      ctx.beginPath()
      ctx.moveTo(x, 0)
      ctx.lineTo(x, h)
      ctx.stroke()
      ctx.setLineDash([])
      ctx.fillStyle = hw.textPrimary
      ctx.font = '9px Inter, ui-sans-serif, sans-serif'
      const label = laneDisplayValue(def, hover.value)
      const tw = ctx.measureText(label).width
      ctx.fillStyle = 'rgba(8,8,12,0.9)'
      ctx.fillRect(x + 4, 2, tw + 6, 14)
      ctx.fillStyle = hw.textPrimary
      ctx.fillText(label, x + 7, 12)
    }
  }, [values, height, scrollX, pixelsPerTick, laneWidthTicks, slotToTick, def, hover])

  useEffect(() => { draw() }, [draw])

  useEffect(() => {
    const obs = new ResizeObserver(() => draw())
    if (containerRef.current) obs.observe(containerRef.current)
    return () => obs.disconnect()
  }, [draw])

  const mouseToSlot = useCallback((clientX: number): number => {
    const rect = canvasRef.current!.getBoundingClientRect()
    const mx = clientX - rect.left
    const tick = (mx + scrollX) / pixelsPerTick
    return tickToSlot(tick)
  }, [scrollX, pixelsPerTick, tickToSlot])

  const mouseToValue = useCallback((clientY: number): number => {
    const rect = canvasRef.current!.getBoundingClientRect()
    const my = clientY - rect.top
    return Math.max(0, Math.min(1, 1 - my / height))
  }, [height])

  const applyLine = useCallback((a: { slot: number; value: number }, b: { slot: number; value: number }) => {
    const from = Math.min(a.slot, b.slot)
    const to = Math.max(a.slot, b.slot)
    const startV = a.slot <= b.slot ? a.value : b.value
    const endV = a.slot <= b.slot ? b.value : a.value
    const span = Math.max(1, to - from)
    const out: number[] = []
    for (let i = 0; i <= to - from; i++) out.push(startV + (endV - startV) * (i / span))
    setValuesRange(clipId, def, from, out)
  }, [clipId, def, setValuesRange])

  const applyCurve = useCallback((p0: { slot: number; value: number }, p1: { slot: number; value: number }, p2: { slot: number; value: number }) => {
    const ordered = [p0, p1, p2].sort((a, b) => a.slot - b.slot)
    const from = ordered[0].slot
    const to = ordered[2].slot
    const span = Math.max(1, to - from)
    const out: number[] = []
    for (let i = 0; i <= to - from; i++) {
      const t = i / span
      const u = 1 - t
      const v = u * u * ordered[0].value + 2 * u * t * ordered[1].value + t * t * ordered[2].value
      out.push(v)
    }
    setValuesRange(clipId, def, from, out)
  }, [clipId, def, setValuesRange])

  const handleMouseDown = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    e.preventDefault()
    const slot = mouseToSlot(e.clientX)
    const value = mouseToValue(e.clientY)

    if (tool === 'pencil') {
      setValueAt(clipId, def, slot, value)
      let lastSlot = slot
      let lastValue = value
      const onMove = (ev: MouseEvent) => {
        const s1 = mouseToSlot(ev.clientX)
        const v1 = mouseToValue(ev.clientY)
        setHover({ slot: s1, value: v1 })
        if (s1 === lastSlot) {
          setValueAt(clipId, def, s1, v1)
        } else {
          const from = Math.min(lastSlot, s1)
          const to = Math.max(lastSlot, s1)
          const startV = lastSlot <= s1 ? lastValue : v1
          const endV = lastSlot <= s1 ? v1 : lastValue
          const span = Math.max(1, to - from)
          const fill: number[] = []
          for (let i = 0; i <= to - from; i++) fill.push(startV + (endV - startV) * (i / span))
          setValuesRange(clipId, def, from, fill)
        }
        lastSlot = s1
        lastValue = v1
      }
      const onUp = () => {
        window.removeEventListener('mousemove', onMove)
        window.removeEventListener('mouseup', onUp)
      }
      window.addEventListener('mousemove', onMove)
      window.addEventListener('mouseup', onUp)
      return
    }

    if (tool === 'line') {
      if (!lineAnchor.current) {
        lineAnchor.current = { slot, value }
        setHover({ slot, value })
      } else {
        applyLine(lineAnchor.current, { slot, value })
        lineAnchor.current = null
      }
      return
    }

    if (tool === 'curve') {
      curveAnchors.current.push({ slot, value })
      if (curveAnchors.current.length === 3) {
        applyCurve(curveAnchors.current[0], curveAnchors.current[1], curveAnchors.current[2])
        curveAnchors.current = []
      }
      return
    }
  }, [tool, mouseToSlot, mouseToValue, setValueAt, setValuesRange, clipId, def, applyLine, applyCurve])

  const handleMouseMove = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    const slot = mouseToSlot(e.clientX)
    const value = mouseToValue(e.clientY)
    setHover({ slot, value })
  }, [mouseToSlot, mouseToValue])

  const handleMouseLeave = useCallback(() => { setHover(null) }, [])

  useEffect(() => {
    lineAnchor.current = null
    curveAnchors.current = []
  }, [tool])

  return (
    <div style={{
      display: 'flex',
      borderTop: `1px solid rgba(255,255,255,0.04)`,
    }}>
      <div style={{
        width: keyboardWidth,
        padding: '4px 6px',
        display: 'flex',
        flexDirection: 'column',
        gap: 3,
        background: '#08080d',
        borderRight: `1px solid rgba(255,255,255,0.04)`,
      }}>
        <select
          value={def.id}
          onChange={(e) => onChangeLane(e.target.value)}
          style={{
            fontSize: 9,
            background: 'rgba(255,255,255,0.04)',
            color: hw.textPrimary,
            border: `1px solid ${hw.border}`,
            borderRadius: 3,
            padding: '2px 3px',
            outline: 'none',
          }}
          title="CC lane type"
        >
          {allDefs.map(d => (
            <option key={d.id} value={d.id}>{d.label}</option>
          ))}
        </select>
        <div style={{ display: 'flex', gap: 3 }}>
          <button
            onClick={onClear}
            title="Clear lane"
            style={laneHeaderBtn}
          >
            Clr
          </button>
          <button
            onClick={onRemove}
            title="Remove lane"
            style={laneHeaderBtn}
          >
            ×
          </button>
        </div>
      </div>
      <div ref={containerRef} style={{ flex: 1, height, overflow: 'hidden' }}>
        <canvas
          ref={canvasRef}
          onMouseDown={handleMouseDown}
          onMouseMove={handleMouseMove}
          onMouseLeave={handleMouseLeave}
          style={{
            display: 'block',
            cursor: tool === 'pencil' ? 'crosshair' : 'cell',
          }}
          title={`${def.label} — ${tool === 'pencil' ? 'click/drag to draw' : tool === 'line' ? 'click two points for line' : 'click three points for curve'}`}
        />
      </div>
    </div>
  )
}

const laneHeaderBtn: React.CSSProperties = {
  flex: 1,
  padding: '2px 0',
  fontSize: 9,
  background: 'transparent',
  color: hw.textSecondary,
  border: `1px solid ${hw.border}`,
  borderRadius: 2,
  cursor: 'pointer',
}
