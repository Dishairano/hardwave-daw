import { memo, useMemo } from 'react'
import { useTrackStore } from '../../../stores/trackStore'
import { useSendStore } from '../../../stores/sendStore'
import { insertName } from '../../../lib/mixerNumbers'
import { t, tf, useLanguage } from '../../../i18n'

/** How many channels the list shows before "and N more". */
const SHOWN = 5

/**
 * What plays into the selected insert, as mockup M2's panel lists it: the
 * channels routed to it (with how many clips each has on the playlist) and
 * the inserts sending to it. With FL's numbers a channel is no strip of its
 * own any more, so this is where an insert says what feeds it. Read only:
 * routing is changed in the rack's insert box or the Output list.
 */
export const PlaysInto = memo(function PlaysInto({ trackId }: { trackId: string }) {
  useLanguage()
  const tracks = useTrackStore((s) => s.tracks)
  const byTrack = useSendStore((s) => s.byTrack)
  const self = tracks.find((tr) => tr.id === trackId)
  const isMaster = self?.kind === 'Master'
  const { from, senders } = useMemo(() => {
    // A channel with an insert of its own (every track of a song made
    // before) plays through its own strip first.
    const own = !isMaster && self?.rackGroup != null && self.kind !== 'Automation' ? [self] : []
    const from = [...own, ...tracks.filter((tr) => tr.id !== trackId && tr.kind !== 'Master' && tr.kind !== 'Automation'
      && (isMaster ? !tr.outputBus && tr.mixerNumber == null : tr.outputBus === trackId))]
    const senders = tracks.filter((tr) => (byTrack[tr.id] ?? []).some((s) => s.target === trackId && s.enabled))
    return { from, senders }
  }, [tracks, byTrack, trackId, isMaster, self])
  if (!self) return null

  const count = [
    from.length === 1 ? t('1 channel') : tf('{n} channels', { n: from.length }),
    ...(senders.length ? [senders.length === 1 ? t('1 send') : tf('{n} sends', { n: senders.length })] : []),
  ].join(' · ')
  return (
    <div className="mx-fx-plays">
      <div className="mx-fx-routing-head">
        <h4>{t('Plays into it')} <span>{count}</span></h4>
      </div>
      {isMaster && (
        <div className="mx-fx-routing-empty">
          {from.length
            ? tf('Every insert, and the {n} channels that play straight into it.', { n: from.length })
            : t('Every insert.')}
        </div>
      )}
      {!isMaster && from.length === 0 && senders.length === 0 && (
        <div className="mx-fx-routing-empty">{t('Nothing yet. It processes no audio until something plays into it.')}</div>
      )}
      {!isMaster && from.slice(0, SHOWN).map((ch) => {
        const clips = ch.clips?.length ?? 0
        return (
          <div key={ch.id} className="mx-fx-plays-row">
            <i style={{ background: /^#[0-9a-f]{6}$/i.test(ch.color) ? ch.color : '#52525b' }} />
            <span title={ch.name}>{ch.name}</span>
            <small>
              {ch.id === trackId
                ? t('its own channel')
                : clips ? (clips === 1 ? t('1 clip') : tf('{n} clips', { n: clips })) : t('no clips on the playlist')}
            </small>
          </div>
        )
      })}
      {!isMaster && from.length > SHOWN && (
        <div className="mx-fx-plays-row more">{tf('and {n} more', { n: from.length - SHOWN })}</div>
      )}
      {senders.map((tr) => (
        <div key={`send-${tr.id}`} className="mx-fx-plays-row">
          <i style={{ background: /^#[0-9a-f]{6}$/i.test(tr.color) ? tr.color : '#52525b' }} />
          <span>{tr.mixerNumber != null ? `${tr.mixerNumber} ${insertName(tr.mixerNumber, tr)}` : tr.name}</span>
          <small>{t('send')}</small>
        </div>
      ))}
    </div>
  )
})
