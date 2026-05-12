import { memo, useEffect, useRef } from 'react'
import { registerMeter, unregisterMeter } from '../../services/meterStream'

export interface MeterProps {
  /**
   * Track to subscribe to. When set, the meter paints itself via a global
   * rAF loop reading the latest meter store value — NO React re-renders
   * per tick. Leave undefined for legacy static use (in which case the
   * meter renders the `peakDb` prop directly).
   */
  trackId?: string
  /** Channel hint for the track meter pair (l = left, r = right, mono = avg). */
  channel?: 'l' | 'r' | 'mono'
  /** Legacy fallback when `trackId` isn't supplied. */
  peakDb?: number
  /** Lower bound of the visible range. Default -60. */
  minDb?: number
  /** Upper bound. Default +6. */
  maxDb?: number
  /** Bar width in px. Default 4. */
  width?: number
  className?: string
}

const MIN = -60
const MAX = 6

function clamp(v: number, lo: number, hi: number): number {
  if (v < lo) return lo
  if (v > hi) return hi
  return v
}

/**
 * Single-channel level meter.
 *
 * Two render modes:
 *  1. **Live (preferred):** `trackId` + `channel` are supplied. The component
 *     creates a `<canvas>`, registers it with `meterStream`, and the global
 *     rAF loop paints it. The component re-renders only on mount / unmount.
 *  2. **Static (fallback):** when `trackId` is omitted, the component
 *     renders the old `--fill` DIV from a `peakDb` prop. Used for the
 *     master output meter and for static demos / tests.
 *
 * Both modes share the same outer DOM so CSS rules don't fork.
 */
export const Meter = memo(function Meter(props: MeterProps) {
  const { trackId, channel = 'mono', peakDb, minDb = MIN, maxDb = MAX, width = 4, className } = props
  const canvasRef = useRef<HTMLCanvasElement | null>(null)

  // Live mode — register/unregister with the meter stream.
  useEffect(() => {
    if (!trackId) return
    const canvas = canvasRef.current
    if (!canvas) return
    registerMeter(canvas, trackId, channel, minDb, maxDb)
    return () => unregisterMeter(canvas)
  }, [trackId, channel, minDb, maxDb])

  // Static fallback fill — only used when trackId is undefined.
  const staticFill =
    peakDb == null
      ? 0
      : ((clamp(peakDb, minDb, maxDb) - minDb) / Math.max(0.001, maxDb - minDb)) * 100

  return (
    <div
      className={'hw-meter' + (className ? ' ' + className : '')}
      data-ch={channel}
      style={
        {
          width,
          ['--fill' as string]: staticFill.toFixed(2) + '%',
        } as React.CSSProperties
      }
      aria-hidden="true"
    >
      {trackId ? (
        <canvas ref={canvasRef} className="hw-meter-canvas" />
      ) : (
        <span className="hw-meter-fill" />
      )}
    </div>
  )
})

export interface MeterPairProps {
  /**
   * Track id for live mode — both L and R subscribe to the same track,
   * each canvas reads its own channel from the meter stream.
   */
  trackId?: string
  /** Legacy peak values for static fallback. */
  peakL?: number
  peakR?: number
  minDb?: number
  maxDb?: number
  channelWidth?: number
  className?: string
}

/**
 * Two flush level meters (L on the left, R on the right) rendered as one
 * unit. Pass `trackId` for live rAF-painted meters, or `peakL`/`peakR`
 * for static fallback.
 */
export const MeterPair = memo(function MeterPair(props: MeterPairProps) {
  const { trackId, peakL, peakR, minDb, maxDb, channelWidth = 4, className } = props
  return (
    <div className={'hw-meter-pair' + (className ? ' ' + className : '')}>
      <Meter
        trackId={trackId}
        channel="l"
        peakDb={peakL}
        minDb={minDb}
        maxDb={maxDb}
        width={channelWidth}
      />
      <Meter
        trackId={trackId}
        channel="r"
        peakDb={peakR}
        minDb={minDb}
        maxDb={maxDb}
        width={channelWidth}
      />
    </div>
  )
})
