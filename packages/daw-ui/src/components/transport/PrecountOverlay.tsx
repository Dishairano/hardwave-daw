import { useEffect, useState } from 'react'
import { hw } from '../../theme'
import { subscribePrecount } from '../../stores/transportStore'

export function PrecountOverlay() {
  const [state, setState] = useState<{ beat: number; total: number } | null>(null)

  useEffect(() => {
    const unsub = subscribePrecount((s) => setState(s))
    return unsub
  }, [])

  if (!state) return null
  const remaining = Math.max(0, state.total - state.beat)

  return (
    <div
      style={{
        position: 'fixed', top: 80, left: '50%', transform: 'translateX(-50%)',
        zIndex: 9999, padding: '14px 24px', pointerEvents: 'none',
        background: 'rgba(12,12,18,0.92)',
        border: `1px solid ${hw.accent}`, borderRadius: hw.radius.lg,
        boxShadow: `0 0 24px ${hw.accentGlow}`,
        display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 4,
      }}
    >
      <div style={{ fontSize: 10, color: hw.textFaint, textTransform: 'uppercase', letterSpacing: 1 }}>
        Pre-count
      </div>
      <div style={{ fontSize: 32, fontWeight: 700, color: hw.accent, fontVariantNumeric: 'tabular-nums' }}>
        {remaining}
      </div>
      <div style={{ fontSize: 9, color: hw.textMuted }}>
        beat {state.beat + 1} / {state.total}
      </div>
    </div>
  )
}
