import { memo } from 'react'
import { useMeterStore } from '../../../stores/meterStore'
import { tf, useLanguage } from '../../../i18n'

/**
 * The master's left/right correlation under its fader: right of the middle
 * when the two sides agree, left when they cancel. Below zero the mix loses
 * level in mono, on a club system or a phone speaker.
 */
export const MasterCorrelation = memo(function MasterCorrelation() {
  useLanguage()
  const c = useMeterStore(s => s.master.correlation ?? 0)
  const value = Math.max(-1, Math.min(1, c))
  return (
    <div
      className={'mx-corr' + (value < 0 ? ' neg' : '')}
      title={tf('Correlation {value}: +1 is mono, 0 is wide, below 0 the sides cancel in mono', { value: value.toFixed(2) })}
    >
      <div className="mx-corr-track">
        <div className="mx-corr-dot" style={{ left: `${((value + 1) / 2) * 100}%` }} />
      </div>
      <span className="mx-corr-num">{(value > 0 ? '+' : '') + value.toFixed(2)}</span>
    </div>
  )
})
