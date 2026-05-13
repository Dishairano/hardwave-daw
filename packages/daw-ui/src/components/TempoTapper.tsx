import { useEffect, useRef, useState } from 'react'
import { hw } from '../theme'
import { useTransportStore } from '../stores/transportStore'

/**
 * Tempo Tapper — FL Studio View → Tempo tapper.
 *
 * The user clicks the big button (or hits Space / Enter while the panel
 * is focused) in time with a reference rhythm; the panel derives a BPM
 * from the rolling-average interval between the last N taps. "Apply"
 * pushes that BPM onto the transport store / engine.
 *
 * Implementation notes:
 *  - We keep the last 8 tap timestamps in a ref-based ring buffer,
 *    averaging the consecutive intervals so a single dropped tap can't
 *    swing the result wildly.
 *  - Outlier rejection: any interval more than 2× the running median is
 *    discarded — handles the "user paused mid-tap" case cleanly.
 *  - Auto-reset after 2.5 s of silence so a fresh tap session starts
 *    from scratch without the user having to click Reset.
 */

const TAP_HISTORY = 8
const RESET_AFTER_MS = 2500
const OUTLIER_FACTOR = 2.0

interface Props {
  onClose: () => void
}

export function TempoTapper({ onClose }: Props) {
  const setBpm = useTransportStore((s) => s.setBpm)
  const currentBpm = useTransportStore((s) => s.bpm)
  const tapsRef = useRef<number[]>([])
  const [derivedBpm, setDerivedBpm] = useState<number | null>(null)
  const [tapCount, setTapCount] = useState(0)

  const tap = () => {
    const now = performance.now()
    const last = tapsRef.current[tapsRef.current.length - 1]
    if (last != null && now - last > RESET_AFTER_MS) {
      tapsRef.current = []
    }
    tapsRef.current.push(now)
    if (tapsRef.current.length > TAP_HISTORY) {
      tapsRef.current.shift()
    }
    setTapCount(tapsRef.current.length)

    // Need at least two taps to compute an interval.
    if (tapsRef.current.length < 2) {
      setDerivedBpm(null)
      return
    }
    const intervals: number[] = []
    for (let i = 1; i < tapsRef.current.length; i++) {
      intervals.push(tapsRef.current[i] - tapsRef.current[i - 1])
    }
    // Reject outliers against the median to handle a dropped tap.
    const sorted = intervals.slice().sort((a, b) => a - b)
    const median = sorted[Math.floor(sorted.length / 2)]
    const kept = intervals.filter((v) => v < median * OUTLIER_FACTOR && v > median / OUTLIER_FACTOR)
    const avg = kept.reduce((s, v) => s + v, 0) / kept.length
    if (avg <= 0) return
    const bpm = 60_000 / avg
    setDerivedBpm(Math.round(bpm * 10) / 10)
  }

  const reset = () => {
    tapsRef.current = []
    setDerivedBpm(null)
    setTapCount(0)
  }

  const apply = () => {
    if (derivedBpm == null) return
    setBpm(derivedBpm)
    onClose()
  }

  // Keyboard: Space / Enter taps; Esc closes.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return
      if (e.code === 'Space' || e.code === 'Enter') {
        e.preventDefault()
        tap()
      } else if (e.code === 'Escape') {
        e.preventDefault()
        onClose()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 200,
        background: 'rgba(0,0,0,0.6)', backdropFilter: 'blur(4px)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        padding: 24,
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 360, background: 'rgba(12,12,16,0.98)',
          border: `1px solid ${hw.borderLight}`,
          borderRadius: hw.radius.lg,
          boxShadow: '0 20px 60px rgba(0,0,0,0.7)',
          overflow: 'hidden',
        }}
      >
        <div style={{
          padding: '12px 16px', display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          background: 'rgba(255,255,255,0.03)', borderBottom: `1px solid ${hw.border}`,
        }}>
          <span style={{ fontSize: 13, fontWeight: 700, color: hw.accent, letterSpacing: 0.5 }}>
            TEMPO TAPPER
          </span>
          <button onClick={onClose} style={{
            background: 'transparent', border: 0, color: hw.textFaint,
            cursor: 'pointer', fontSize: 16, padding: 4,
          }} aria-label="Close">×</button>
        </div>
        <div style={{ padding: 24, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 16 }}>
          <div style={{ fontSize: 10, color: hw.textMuted, letterSpacing: 0.6, textTransform: 'uppercase' }}>
            Tap in time with the beat — Space or Enter also works
          </div>
          <button
            onClick={tap}
            style={{
              width: 200, height: 100,
              fontSize: 36, fontWeight: 700,
              background: `linear-gradient(180deg, ${hw.accent}, ${hw.accentDim ?? hw.accent})`,
              color: '#fff',
              border: 'none',
              borderRadius: hw.radius.lg,
              cursor: 'pointer',
              fontFamily: 'monospace',
              boxShadow: `0 0 30px ${hw.accent}30, 0 8px 20px rgba(0,0,0,0.5)`,
            }}
          >
            TAP
          </button>
          <div style={{ display: 'flex', gap: 24, alignItems: 'baseline' }}>
            <div style={{ textAlign: 'center' }}>
              <div style={{ fontSize: 30, fontWeight: 700, color: derivedBpm ? hw.textPrimary : hw.textFaint, fontFamily: 'monospace' }}>
                {derivedBpm != null ? derivedBpm.toFixed(1) : '—'}
              </div>
              <div style={{ fontSize: 9, color: hw.textMuted, letterSpacing: 0.4, textTransform: 'uppercase', marginTop: 2 }}>
                Derived BPM
              </div>
            </div>
            <div style={{ textAlign: 'center' }}>
              <div style={{ fontSize: 18, color: hw.textSecondary, fontFamily: 'monospace' }}>
                {currentBpm.toFixed(1)}
              </div>
              <div style={{ fontSize: 9, color: hw.textMuted, letterSpacing: 0.4, textTransform: 'uppercase', marginTop: 2 }}>
                Current
              </div>
            </div>
            <div style={{ textAlign: 'center' }}>
              <div style={{ fontSize: 18, color: hw.textSecondary, fontFamily: 'monospace' }}>
                {tapCount}
              </div>
              <div style={{ fontSize: 9, color: hw.textMuted, letterSpacing: 0.4, textTransform: 'uppercase', marginTop: 2 }}>
                Taps
              </div>
            </div>
          </div>
          <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
            <button onClick={reset} style={{
              padding: '6px 14px', fontSize: 11, fontWeight: 600,
              background: 'rgba(255,255,255,0.08)', color: hw.textSecondary,
              border: `1px solid ${hw.borderDark}`, borderRadius: hw.radius.sm,
              cursor: 'pointer', fontFamily: 'inherit',
            }}>Reset</button>
            <button
              onClick={apply}
              disabled={derivedBpm == null}
              style={{
                padding: '6px 14px', fontSize: 11, fontWeight: 600,
                background: derivedBpm != null ? hw.accent : 'rgba(255,255,255,0.04)',
                color: derivedBpm != null ? '#fff' : hw.textFaint,
                border: `1px solid ${derivedBpm != null ? hw.accent : hw.borderDark}`,
                borderRadius: hw.radius.sm,
                cursor: derivedBpm != null ? 'pointer' : 'not-allowed',
                fontFamily: 'inherit',
              }}
            >Apply to transport</button>
          </div>
        </div>
      </div>
    </div>
  )
}
