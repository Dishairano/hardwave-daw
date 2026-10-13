/**
 * Flat building blocks for the DAW's own windows: knob, panel, check box,
 * select, on/off switch, number box and the hint that follows the mouse.
 * One look for all of them (flat.css), first used by the sample window.
 */
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import './flat.css'

// ------------------------------------------------------------------ hint

/** A hint next to the mouse, while `text` is set. */
export function FlatTip({ text, x, y }: { text: string | null; x: number; y: number }) {
  if (!text) return null
  return createPortal(
    <div className="hwf-tip" style={{ left: x + 14, top: y - 28 }}>{text}</div>,
    document.body,
  )
}

function useTip() {
  const [tip, setTip] = useState<{ text: string; x: number; y: number } | null>(null)
  return {
    tip,
    show: (text: string, e: { clientX: number; clientY: number }) => setTip({ text, x: e.clientX, y: e.clientY }),
    hide: () => setTip(null),
  }
}

// ------------------------------------------------------------------ knob

const ACCENT = '#dc2626'

function drawKnob(canvas: HTMLCanvasElement, size: number, t: number, t0: number, changed: boolean) {
  const dpr = window.devicePixelRatio || 1
  canvas.width = size * dpr
  canvas.height = size * dpr
  const g = canvas.getContext('2d')
  if (!g) return
  g.scale(dpr, dpr)
  const c = size / 2
  const r = size / 2 - 2
  const body = r - 4
  const a0 = Math.PI * 0.75
  const sweep = Math.PI * 1.5
  // The track the value runs along, and the value in red once it is changed.
  g.lineCap = 'round'
  g.lineWidth = 2.5
  g.strokeStyle = 'rgba(255,255,255,.08)'
  g.beginPath()
  g.arc(c, c, r, a0, a0 + sweep)
  g.stroke()
  if (changed) {
    const from = a0 + sweep * t0
    const to = a0 + sweep * t
    g.strokeStyle = ACCENT
    g.beginPath()
    if (to >= from) g.arc(c, c, r, from, to)
    else g.arc(c, c, r, to, from)
    g.stroke()
  }
  g.fillStyle = '#232329'
  g.beginPath()
  g.arc(c, c, body, 0, Math.PI * 2)
  g.fill()
  g.strokeStyle = 'rgba(255,255,255,.10)'
  g.lineWidth = 1
  g.stroke()
  const a = a0 + sweep * t
  g.strokeStyle = '#ececf0'
  g.lineWidth = 2
  g.beginPath()
  g.moveTo(c + Math.cos(a) * body * 0.25, c + Math.sin(a) * body * 0.25)
  g.lineTo(c + Math.cos(a) * (body - 2), c + Math.sin(a) * (body - 2))
  g.stroke()
}

export interface FlatKnobProps {
  label: string
  /** Where the knob is, 0..1. */
  value: number
  /** Where it sits by default, 0..1: the ring shows from here, and only
   *  when the knob is away from it. */
  def: number
  /** What the hint says. */
  text: string
  /** A move: the new place, 0..1, and whether the gesture is over. */
  onChange?: (value: number, done: boolean) => void
  /** Double-click: back to its default. */
  onReset?: () => void
  /** Off: shown, but cannot be turned, with `offText` as its hint. */
  disabled?: boolean
  offText?: string
  size?: number
}

/** A knob: drag up or down (Shift or Ctrl for fine), scroll, double-click
 *  to reset. Its value as text follows the mouse. */
export function FlatKnob({ label, value, def, text, onChange, onReset, disabled, offText, size = 36 }: FlatKnobProps) {
  const ref = useRef<HTMLCanvasElement>(null)
  const { tip, show, hide } = useTip()
  const dragging = useRef(false)
  // The hint follows the value as it changes, so it reads the latest text.
  const textRef = useRef(text)
  textRef.current = text
  const clamp = (v: number) => Math.max(0, Math.min(1, v))

  useLayoutEffect(() => {
    if (ref.current) drawKnob(ref.current, size, clamp(value), clamp(def), Math.abs(value - def) > 1e-6)
  }, [value, def, size])

  const hint = disabled ? (offText ?? `${label}: not in this version yet`) : text

  return (
    <div
      className={`hwf-knob${disabled ? ' is-off' : ''}`}
      onPointerDown={(e) => {
        if (disabled || !onChange) return
        e.preventDefault()
        const target = e.currentTarget
        target.setPointerCapture(e.pointerId)
        dragging.current = true
        const y0 = e.clientY
        const v0 = value
        const move = (ev: PointerEvent) => {
          const fine = ev.shiftKey || ev.ctrlKey
          onChange(clamp(v0 + (y0 - ev.clientY) / (fine ? 700 : 160)), false)
          requestAnimationFrame(() => show(textRef.current, ev))
        }
        const up = (ev: PointerEvent) => {
          dragging.current = false
          target.removeEventListener('pointermove', move)
          target.removeEventListener('pointerup', up)
          onChange(clamp(v0 + (y0 - ev.clientY) / (ev.shiftKey || ev.ctrlKey ? 700 : 160)), true)
          hide()
        }
        target.addEventListener('pointermove', move)
        target.addEventListener('pointerup', up)
      }}
      onDoubleClick={() => { if (!disabled) onReset?.() }}
      onWheel={(e) => {
        if (disabled || !onChange) return
        onChange(clamp(value - Math.sign(e.deltaY) / 60), true)
        const at = { clientX: e.clientX, clientY: e.clientY }
        requestAnimationFrame(() => show(textRef.current, at))
      }}
      onPointerMove={(e) => { if (!dragging.current) show(hint, e) }}
      onPointerLeave={() => { if (!dragging.current) hide() }}
    >
      <canvas ref={ref} style={{ width: size, height: size }} />
      <span className="hwf-lab">{label}</span>
      <FlatTip text={tip?.text ?? null} x={tip?.x ?? 0} y={tip?.y ?? 0} />
    </div>
  )
}

// ------------------------------------------------------------------ panel, check, select, switch, box

export function FlatPanel({ title, children, grow, style }: { title?: string; children: ReactNode; grow?: boolean; style?: React.CSSProperties }) {
  return (
    <div className="hwf-panel" style={{ flex: grow ? 1 : undefined, ...style }}>
      {title && <div className="hwf-panel-title">{title}</div>}
      <div className="hwf-panel-body">{children}</div>
    </div>
  )
}

export function FlatCheck({ label, on, onToggle, disabled, hint }: { label: string; on: boolean; onToggle?: () => void; disabled?: boolean; hint?: string }) {
  const { tip, show, hide } = useTip()
  return (
    <button
      type="button"
      className={`hwf-check${on ? ' is-on' : ''}`}
      disabled={disabled || !onToggle}
      onClick={onToggle}
      onPointerMove={(e) => { if (hint) show(hint, e) }}
      onPointerLeave={hide}
    >
      <i />{label}
      <FlatTip text={tip?.text ?? null} x={tip?.x ?? 0} y={tip?.y ?? 0} />
    </button>
  )
}

export function FlatSelect<T extends string>({ value, options, onPick, disabled, label, style }: {
  value: T
  options: readonly T[]
  onPick?: (v: T) => void
  disabled?: boolean
  /** How a value reads in the list and the box, when not the value itself. */
  label?: (v: T) => string
  style?: React.CSSProperties
}) {
  const [open, setOpen] = useState(false)
  const box = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const away = (e: PointerEvent) => { if (!box.current?.contains(e.target as Node)) setOpen(false) }
    window.addEventListener('pointerdown', away)
    return () => window.removeEventListener('pointerdown', away)
  }, [open])
  const name = (v: T) => (label ? label(v) : v)
  return (
    <div ref={box} style={{ position: 'relative', minWidth: 0, ...style }}>
      <button type="button" className="hwf-select" style={{ width: '100%' }} disabled={disabled || !onPick} onClick={() => setOpen(!open)}>
        <span>{name(value)}</span>
      </button>
      {open && (
        <div className="hwf-menu">
          {options.map((o) => (
            <button type="button" key={o} className={o === value ? 'is-on' : ''} onClick={() => { setOpen(false); onPick?.(o) }}>{name(o)}</button>
          ))}
        </div>
      )}
    </div>
  )
}

export function FlatSwitch({ label, on, onToggle }: { label: string; on: boolean; onToggle: () => void }) {
  return (
    <button type="button" className={`hwf-switch${on ? '' : ' is-off'}`} onClick={onToggle} title={on ? 'On' : 'Off'}>
      <i /><span className="hwf-lab">{label}</span>
    </button>
  )
}

/** A number in a box with its label under it; scroll it when `onWheel` is set. */
export function FlatBox({ label, value, small, onStep, hint }: { label: string; value: ReactNode; small?: string; onStep?: (by: number) => void; hint?: string }) {
  const { tip, show, hide } = useTip()
  return (
    <div
      className={`hwf-box${onStep ? ' is-wheel' : ''}`}
      onWheel={(e) => { if (onStep) onStep(e.deltaY < 0 ? 1 : -1) }}
      onPointerMove={(e) => { if (hint) show(hint, e) }}
      onPointerLeave={hide}
    >
      <div>{value}{small && <small>{small}</small>}</div>
      <span className="hwf-lab">{label}</span>
      <FlatTip text={tip?.text ?? null} x={tip?.x ?? 0} y={tip?.y ?? 0} />
    </div>
  )
}
