import { useCallback, useRef, useState } from 'react'

export interface FaderProps {
  /** Current value in dB (typical range -∞ .. +6). */
  valueDb: number
  /** Lower bound of the visible range. Default -∞ behavior via `silenceDb`. */
  minDb?: number
  /** Upper bound of the visible range. */
  maxDb?: number
  /** Visual unity-gain marker (rendered as a tick line). */
  unityDb?: number
  /** Below this dB the fader visually pins to the bottom (silence). */
  silenceDb?: number
  /** Pixels of vertical drag per dB. Default 6. Higher = touchier. */
  sensitivity?: number
  /** Track height in px. The cap rides over the full track. */
  height?: number | string
  onChange: (db: number) => void
  /** Called once when interaction starts (snapshot for optimistic-local pattern). */
  onChangeStart?: () => void
  /** Called once when interaction ends (commit to backend). */
  onChangeEnd?: (final: number) => void
  /** Hover/title text. */
  title?: string
  className?: string
}

const DEFAULT_MIN = -60
const DEFAULT_MAX = 6
const DEFAULT_UNITY = 0
const DEFAULT_SILENCE = -60
const DEFAULT_SENSITIVITY = 6

function clamp(v: number, lo: number, hi: number): number {
  if (v < lo) return lo
  if (v > hi) return hi
  return v
}

/**
 * Vertical fader. Visual cap position is driven by the `--cap-top` CSS
 * variable so dragging updates a single style without forcing React to
 * reconcile children. Controlled component — `valueDb` is the source of
 * truth, parent owns commit cadence via `onChangeStart` / `onChangeEnd`.
 *
 * Inputs handled:
 * - Vertical drag (pointer): up = louder, down = quieter.
 *   Shift = fine (×0.25). Ctrl/⌘ = coarse (×4).
 * - Wheel: vertical, ctrl/⌘ for fine. Debounced commit on stop.
 * - Double-click: reset to `unityDb`.
 */
export function Fader(props: FaderProps) {
  const {
    valueDb,
    minDb = DEFAULT_MIN,
    maxDb = DEFAULT_MAX,
    unityDb = DEFAULT_UNITY,
    silenceDb = DEFAULT_SILENCE,
    sensitivity = DEFAULT_SENSITIVITY,
    height,
    onChange,
    onChangeStart,
    onChangeEnd,
    title,
    className,
  } = props

  const ref = useRef<HTMLDivElement | null>(null)
  const wheelTimer = useRef<number | null>(null)
  const [dragging, setDragging] = useState(false)
  const [editing, setEditing] = useState<string | null>(null)

  // Map dB → top%. minDb at the bottom = 100%, maxDb at top = 0%.
  // Below silenceDb we pin to 100% so the cap reads as "off".
  const effective = clamp(valueDb, silenceDb, maxDb)
  const range = maxDb - minDb
  const fromTop = range > 0 ? (1 - (effective - minDb) / range) * 100 : 50
  const capTop = clamp(fromTop, 0, 100)

  const handlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      e.preventDefault()
      const el = ref.current
      if (!el) return
      el.setPointerCapture(e.pointerId)
      setDragging(true)
      onChangeStart?.()

      const startY = e.clientY
      const startDb = valueDb

      // FL Studio convention: Ctrl = fine, plain = normal.
      const onMove = (ev: PointerEvent) => {
        const dy = startY - ev.clientY
        const fine = ev.ctrlKey || ev.metaKey ? 0.25 : 1
        const nextDb = clamp(startDb + (dy / sensitivity) * fine, minDb, maxDb)
        onChange(nextDb)
      }
      const onUp = (ev: PointerEvent) => {
        el.releasePointerCapture(ev.pointerId)
        el.removeEventListener('pointermove', onMove)
        el.removeEventListener('pointerup', onUp)
        el.removeEventListener('pointercancel', onUp)
        setDragging(false)
        const dy = startY - ev.clientY
        const fine = ev.ctrlKey || ev.metaKey ? 0.25 : 1
        const finalDb = clamp(startDb + (dy / sensitivity) * fine, minDb, maxDb)
        onChangeEnd?.(finalDb)
      }
      el.addEventListener('pointermove', onMove)
      el.addEventListener('pointerup', onUp)
      el.addEventListener('pointercancel', onUp)
    },
    [valueDb, sensitivity, minDb, maxDb, onChange, onChangeStart, onChangeEnd],
  )

  const handleWheel = useCallback(
    (e: React.WheelEvent<HTMLDivElement>) => {
      e.preventDefault()
      const dir = e.deltaY < 0 ? 1 : -1
      const fine = e.ctrlKey || e.metaKey ? 0.25 : 1
      const stepDb = 1 * fine // 1 dB per notch (fine = 0.25 dB)
      const next = clamp(valueDb + dir * stepDb, minDb, maxDb)
      if (wheelTimer.current == null) onChangeStart?.()
      else window.clearTimeout(wheelTimer.current)
      onChange(next)
      wheelTimer.current = window.setTimeout(() => {
        wheelTimer.current = null
        onChangeEnd?.(next)
      }, 220)
    },
    [valueDb, minDb, maxDb, onChange, onChangeStart, onChangeEnd],
  )

  const handleDoubleClick = useCallback(() => {
    const target = clamp(unityDb, minDb, maxDb)
    onChangeStart?.()
    onChange(target)
    onChangeEnd?.(target)
  }, [unityDb, minDb, maxDb, onChange, onChangeStart, onChangeEnd])

  const handleContextMenu = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault()
      const seed = Number.isFinite(valueDb) ? valueDb.toFixed(1) : '0'
      setEditing(seed)
    },
    [valueDb],
  )

  const commitEdit = useCallback(
    (raw: string) => {
      // Accept "-inf", "inf", "-∞" as silence to match the dB readout's
      // visual convention.
      const trimmed = raw.trim().toLowerCase()
      if (trimmed === '-inf' || trimmed === '-∞' || trimmed === '-infinity') {
        onChangeStart?.()
        onChange(minDb)
        onChangeEnd?.(minDb)
        return
      }
      const parsed = parseFloat(raw)
      if (!Number.isFinite(parsed)) return
      const clamped = clamp(parsed, minDb, maxDb)
      onChangeStart?.()
      onChange(clamped)
      onChangeEnd?.(clamped)
    },
    [minDb, maxDb, onChange, onChangeStart, onChangeEnd],
  )

  return (
    <div
      ref={ref}
      className={'hw-fader' + (dragging ? ' dragging' : '') + (className ? ' ' + className : '')}
      style={
        {
          height,
          ['--cap-top' as string]: capTop.toFixed(2) + '%',
        } as React.CSSProperties
      }
      onPointerDown={editing == null ? handlePointerDown : undefined}
      onWheel={editing == null ? handleWheel : undefined}
      onDoubleClick={editing == null ? handleDoubleClick : undefined}
      onContextMenu={handleContextMenu}
      role="slider"
      aria-valuemin={minDb}
      aria-valuemax={maxDb}
      aria-valuenow={valueDb}
      aria-label={title ?? 'volume'}
      tabIndex={0}
      title={title}
    >
      <span className="hw-fader-track" aria-hidden="true" />
      <span className="hw-fader-cap" aria-hidden="true" />
      {editing != null && (
        <input
          type="text"
          inputMode="numeric"
          className="hw-fader-edit"
          value={editing}
          autoFocus
          onChange={(e) => setEditing(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              commitEdit(editing)
              setEditing(null)
              e.preventDefault()
            } else if (e.key === 'Escape') {
              setEditing(null)
              e.preventDefault()
            }
            e.stopPropagation()
          }}
          onBlur={() => {
            commitEdit(editing)
            setEditing(null)
          }}
          onClick={(e) => e.stopPropagation()}
          onPointerDown={(e) => e.stopPropagation()}
        />
      )}
    </div>
  )
}
