import { memo } from 'react'

export interface SendsDotsProps {
  /**
   * Up to 5 send slot states. `false` = inactive (dim dot), `'post'` =
   * post-fader send (green), `'pre'` = pre-fader / reverb-bound (blue).
   * Shorter arrays are zero-padded to 5 slots.
   */
  sends: Array<false | 'post' | 'pre'>
  className?: string
}

const MAX_DOTS = 5

/**
 * 5-LED indicator row at the bottom of a strip showing which buses/returns
 * this track sends to. Visual only — clicking a strip's send routing
 * happens in the FX rack panel on the right, not here.
 */
export const SendsDots = memo(function SendsDots(props: SendsDotsProps) {
  const { sends, className } = props
  const padded: Array<false | 'post' | 'pre'> = []
  for (let i = 0; i < MAX_DOTS; i++) padded.push(sends[i] ?? false)
  return (
    <div className={'hw-sends-dots' + (className ? ' ' + className : '')} aria-hidden="true">
      {padded.map((state, i) => (
        <span
          key={i}
          className={
            'hw-sends-dot' +
            (state === 'post' ? ' on' : '') +
            (state === 'pre' ? ' on pre' : '')
          }
        />
      ))}
    </div>
  )
})
