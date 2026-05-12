import { useCallback, useEffect, useRef, useState } from 'react'

export type KnobKind = 'pan' | 'width' | 'wet' | 'gain' | 'generic'

export interface KnobProps {
  /** Current value, mapped to the knob's full sweep. */
  value: number
  /** Inclusive lower bound of `value`. */
  min: number
  /** Inclusive upper bound of `value`. */
  max: number
  /** Value applied on double-click. Defaults to (min+max)/2 for bipolar knobs. */
  defaultValue?: number
  /** Drives the value-tooltip label. */
  kind?: KnobKind
  /** Indicator sweep in degrees from center; defaults to 135 (±135°). */
  sweep?: number
  /** Pixels of vertical drag per degree of rotation. Smaller = touchier. */
  sensitivity?: number
  /** Side length in px. Default 24. */
  size?: number
  /** Called with the new clamped value while the user drags or wheels. */
  onChange: (next: number) => void
  /**
   * Called once when interaction starts (pointerdown). Use to snapshot state
   * for an optimistic-local pattern (so we don't IPC on every pixel).
   */
  onChangeStart?: () => void
  /**
   * Called once when interaction ends (pointerup, wheel-debounce, dblclick).
   * Use to commit the final value to the backend.
   */
  onChangeEnd?: (final: number) => void
  /** Accessibility / hover text. */
  title?: string
  className?: string
}

const WHEEL_COMMIT_DEBOUNCE_MS = 220

function labelFor(kind: KnobKind, val: number, min: number, max: number): string {
  // Map raw value into a -100..+100 normalized form for bipolar kinds so the
  // tooltip always reads in a familiar unit. Pan: L100/C/R100. Width: M100/ST/W100.
  // Wet/gain use the raw value.
  const bipolar = kind === 'pan' || kind === 'width'
  if (bipolar) {
    const mid = (min + max) / 2
    const span = (max - min) / 2 || 1
    const norm = Math.round(((val - mid) / span) * 100)
    if (kind === 'pan') {
      if (norm === 0) return 'C'
      return (norm < 0 ? 'L' : 'R') + Math.abs(norm)
    }
    // width
    if (norm === 0) return 'ST'
    return (norm < 0 ? 'M' : 'W') + Math.abs(norm)
  }
  if (kind === 'wet') return Math.round(val) + '%'
  if (kind === 'gain') return val.toFixed(1) + ' dB'
  return val.toFixed(2)
}

function clamp(v: number, lo: number, hi: number): number {
  if (v < lo) return lo
  if (v > hi) return hi
  return v
}

/**
 * Turnable rotary knob. The visual indicator rotates via the `--rot` CSS
 * variable so the indicator update is a single-style mutation, not a React
 * re-render. The component is still controlled — `value` is the source of
 * truth, and parents should pair it with `onChangeStart` / `onChangeEnd` to
 * batch backend commits.
 *
 * Inputs handled:
 * - Vertical drag (pointer): up = increase, down = decrease.
 * - Wheel: up notch = increase, down notch = decrease. Ctrl/⌘ = fine (×0.25).
 * - Shift held during drag = fine (×0.25). Ctrl/⌘ during drag = coarse (×4).
 * - Double-click: reset to `defaultValue` (or midpoint for bipolar kinds).
 */
export function Knob(props: KnobProps) {
  const {
    value,
    min,
    max,
    defaultValue,
    kind = 'generic',
    sweep = 135,
    sensitivity = 1.5,
    size = 24,
    onChange,
    onChangeStart,
    onChangeEnd,
    title,
    className,
  } = props

  const ref = useRef<HTMLDivElement | null>(null)
  const wheelCommitTimer = useRef<number | null>(null)
  const [dragging, setDragging] = useState(false)

  // Normalize value → degrees for the indicator.
  const mid = (min + max) / 2
  const span = (max - min) / 2 || 1
  const norm = (value - mid) / span // -1..+1
  const deg = clamp(norm * sweep, -sweep, sweep)

  const valueFromDegrees = useCallback(
    (d: number): number => {
      const n = clamp(d / sweep, -1, 1)
      return clamp(mid + n * span, min, max)
    },
    [mid, span, min, max, sweep],
  )

  const handlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      e.preventDefault()
      const el = ref.current
      if (!el) return
      el.setPointerCapture(e.pointerId)
      setDragging(true)
      onChangeStart?.()

      const startY = e.clientY
      const startDeg = deg

      const onMove = (ev: PointerEvent) => {
        const dy = startY - ev.clientY // up = increase
        const fine = ev.shiftKey ? 0.25 : ev.ctrlKey || ev.metaKey ? 4 : 1
        const nextDeg = startDeg + (dy / sensitivity) * fine
        onChange(valueFromDegrees(nextDeg))
      }
      const onUp = (ev: PointerEvent) => {
        el.releasePointerCapture(ev.pointerId)
        el.removeEventListener('pointermove', onMove)
        el.removeEventListener('pointerup', onUp)
        el.removeEventListener('pointercancel', onUp)
        setDragging(false)
        // Read the latest committed deg from the rotation we just applied —
        // the parent's render flips `value`, but onChangeEnd needs the value
        // we landed on. Re-derive from the live event for correctness.
        const finalDy = startY - ev.clientY
        const fine = ev.shiftKey ? 0.25 : ev.ctrlKey || ev.metaKey ? 4 : 1
        const finalDeg = startDeg + (finalDy / sensitivity) * fine
        onChangeEnd?.(valueFromDegrees(finalDeg))
      }
      el.addEventListener('pointermove', onMove)
      el.addEventListener('pointerup', onUp)
      el.addEventListener('pointercancel', onUp)
    },
    [deg, sensitivity, onChange, onChangeStart, onChangeEnd, valueFromDegrees],
  )

  const handleWheel = useCallback(
    (e: React.WheelEvent<HTMLDivElement>) => {
      e.preventDefault()
      const dir = e.deltaY < 0 ? 1 : -1
      const fine = e.ctrlKey || e.metaKey ? 0.25 : 1
      const stepDeg = (4 / 100) * sweep * fine // 4 normalized units per notch
      const nextDeg = clamp(deg + dir * stepDeg, -sweep, sweep)
      onChange(valueFromDegrees(nextDeg))
      // Debounce a commit after the user stops scrolling so we still
      // batch the IPC. First wheel event triggers onChangeStart once.
      if (wheelCommitTimer.current == null) onChangeStart?.()
      else window.clearTimeout(wheelCommitTimer.current)
      wheelCommitTimer.current = window.setTimeout(() => {
        wheelCommitTimer.current = null
        onChangeEnd?.(valueFromDegrees(nextDeg))
      }, WHEEL_COMMIT_DEBOUNCE_MS)
    },
    [deg, sweep, onChange, onChangeStart, onChangeEnd, valueFromDegrees],
  )

  const handleDoubleClick = useCallback(() => {
    const target = defaultValue ?? mid
    const clamped = clamp(target, min, max)
    onChangeStart?.()
    onChange(clamped)
    onChangeEnd?.(clamped)
  }, [defaultValue, mid, min, max, onChange, onChangeStart, onChangeEnd])

  // Cleanup on unmount — kill any pending wheel commit.
  useEffect(() => {
    return () => {
      if (wheelCommitTimer.current != null) {
        window.clearTimeout(wheelCommitTimer.current)
      }
    }
  }, [])

  return (
    <div
      ref={ref}
      className={'hw-knob' + (dragging ? ' dragging' : '') + (className ? ' ' + className : '')}
      style={
        {
          width: size,
          height: size,
          ['--rot' as string]: deg.toFixed(1) + 'deg',
        } as React.CSSProperties
      }
      onPointerDown={handlePointerDown}
      onWheel={handleWheel}
      onDoubleClick={handleDoubleClick}
      role="slider"
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      aria-label={title ?? kind}
      tabIndex={0}
      title={title}
    >
      <span className="hw-knob-indicator" aria-hidden="true" />
      <span className="hw-knob-tooltip" aria-hidden="true">
        {labelFor(kind, value, min, max)}
      </span>
    </div>
  )
}
