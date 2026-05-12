import { memo } from 'react'

export interface MeterProps {
  /** Latest peak value in dB. -∞ for silent (drawn as zero height). */
  peakDb: number
  /** Lower bound of the visible range (silence at this point). Default -60. */
  minDb?: number
  /** Upper bound. Default +6. */
  maxDb?: number
  /** Channel hint — used only for the optional `data-ch` selector hook. */
  channel?: 'l' | 'r' | 'mono'
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
 * Phase 1 implementation: DIV-driven height controlled via the `--fill` CSS
 * variable. The component is `React.memo` so it only re-renders when its
 * input dB actually changes — but at 60 Hz across 500 strips that's still
 * 30 k re-renders / sec. Phase 4 swaps this body for a `<canvas>` painted
 * from a single global rAF loop reading shared meter state.
 *
 * The gradient (green → amber → red) lives in CSS so the JS only sets the
 * height percentage. Bottom-anchored fill so peaks rise upward.
 */
export const Meter = memo(function Meter(props: MeterProps) {
  const { peakDb, minDb = MIN, maxDb = MAX, channel, width = 4, className } = props
  const range = maxDb - minDb
  const clamped = clamp(peakDb, minDb, maxDb)
  const fillPct = range > 0 ? ((clamped - minDb) / range) * 100 : 0
  return (
    <div
      className={'hw-meter' + (className ? ' ' + className : '')}
      data-ch={channel}
      style={{ width, ['--fill' as string]: fillPct.toFixed(2) + '%' } as React.CSSProperties}
      aria-hidden="true"
    >
      <span className="hw-meter-fill" />
    </div>
  )
})

export interface MeterPairProps {
  /** Left channel peak in dB. */
  peakL: number
  /** Right channel peak in dB. */
  peakR: number
  /** Lower / upper bounds. */
  minDb?: number
  maxDb?: number
  /** Per-channel bar width. Default 4 each. */
  channelWidth?: number
  className?: string
}

/**
 * Two flush level meters (L on the left, R on the right) rendered as one
 * unit. Used inside `FaderMeter` so L+R reads as a single stereo strip
 * next to the fader column.
 */
export const MeterPair = memo(function MeterPair(props: MeterPairProps) {
  const { peakL, peakR, minDb, maxDb, channelWidth = 4, className } = props
  return (
    <div className={'hw-meter-pair' + (className ? ' ' + className : '')}>
      <Meter peakDb={peakL} minDb={minDb} maxDb={maxDb} channel="l" width={channelWidth} />
      <Meter peakDb={peakR} minDb={minDb} maxDb={maxDb} channel="r" width={channelWidth} />
    </div>
  )
})
