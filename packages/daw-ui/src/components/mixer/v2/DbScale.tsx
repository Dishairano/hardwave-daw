import { memo } from 'react'

export interface DbScaleProps {
  className?: string
}

/**
 * dB scale column for the FaderMeter group. Pure visual — labels are
 * fixed, ticks are rendered via the CSS `i.t1 / .t2 / .t3` selectors so
 * the component never re-renders. Placed between the fader and the L+R
 * meter pair so the eye reads one consistent scale across both.
 *
 * 0 dB tick is highlighted in the accent color to read as the ceiling.
 */
export const DbScale = memo(function DbScale(props: DbScaleProps) {
  const { className } = props
  return (
    <div className={'hw-db-scale' + (className ? ' ' + className : '')} aria-hidden="true">
      <span className="hw-db-zero">0</span>
      <span>-6</span>
      <span>-12</span>
      <span>-24</span>
      <span>-48</span>
      <i className="hw-db-tick hw-db-t1" />
      <i className="hw-db-tick hw-db-t2" />
      <i className="hw-db-tick hw-db-t3" />
    </div>
  )
})
