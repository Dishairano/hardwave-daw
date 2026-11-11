import { memo, useCallback } from 'react'
import { useMixerSelectionStore } from '../../../stores/mixerSelectionStore'
import { StripName } from './StripName'
import { t, tf, useLanguage } from '../../../i18n'

/**
 * An insert number with no track behind it, drawn dimmed as in mockup M2:
 * its number, "Insert N", and an empty rail where the fader would be. It
 * processes no audio and costs nothing. It becomes a full strip when a
 * channel is routed to it (the rack's insert box, the routing panel), an
 * effect is put on it (select it, then a slot in the FX panel) or it is
 * given a name.
 */
export const EmptyStrip = memo(function EmptyStrip({ number, selected }: { number: number; selected: boolean }) {
  useLanguage()
  const onClick = useCallback(() => useMixerSelectionStore.getState().selectEmptyNumber(number), [number])
  const onContextMenu = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    useMixerSelectionStore.getState().selectEmptyNumber(number)
    useMixerSelectionStore.getState().openStripMenu({ number, x: e.clientX, y: e.clientY })
  }, [number])
  return (
    <div
      className={'mx-strip empty' + (selected ? ' selected' : '')}
      onClick={onClick}
      onContextMenu={onContextMenu}
      data-idx={number}
      data-empty="1"
      title={tf('Insert {n} is empty: nothing plays into it. Route a channel here or put an effect on it to use it.', { n: number })}
    >
      <div className="mx-s-num">
        <span className="mx-s-num-idx">{number}</span>
      </div>
      <StripName number={number} trackId={null} name="" />
      <div className="mx-s-empty-note">{t('empty')}</div>
      <div className="mx-s-empty-rail"><span /></div>
      <div className="mx-s-db" />
      <div className="mx-s-out" />
    </div>
  )
})
