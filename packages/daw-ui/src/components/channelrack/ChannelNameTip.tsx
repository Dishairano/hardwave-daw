import { createPortal } from 'react-dom'
import type { TrackInfo } from '../../stores/trackStore'
import { channelSamplePath, SAMPLER_ID } from '../../stores/instrumentChannels'
import { t, tf } from '../../i18n'

/** What the tooltip on a channel's name says: what plays, and what a click
 *  and a drop do. Null for a channel that is not an instrument channel. */
export interface ChannelTip {
  name: string
  what: string
  how: string
}

const HEAR = 'Click a step or the name to hear it.'

export function channelTip(track: TrackInfo, instrumentName?: string): ChannelTip | null {
  if (track.kind !== 'Midi') return null
  const name = track.name
  const sample = channelSamplePath(track)
  if (sample) {
    const file = sample.split(/[\\/]/).pop() || sample
    return {
      name,
      what: tf('{file} in Hardwave Sampler.', { file }),
      how: `${t(HEAR)} ${t('Drop another sample on the channel to swap it.')}`,
    }
  }
  if (track.inserts?.some(s => s.pluginId === SAMPLER_ID)) {
    return { name, what: t('Hardwave Sampler, with no sample yet.'), how: t('Drop a sample on the channel to load it.') }
  }
  if (instrumentName) return { name, what: `${instrumentName}.`, how: t(HEAR) }
  if (track.instrument === 'kick_synth') {
    return { name, what: t('the kick synth.'), how: `${t(HEAR)} ${t('Double-click the name for its editor.')}` }
  }
  const empty = !(track.inserts?.length)
  return {
    name,
    what: t('the built-in synth.'),
    how: empty ? `${t(HEAR)} ${t('Drop a sample on the channel to play it instead.')}` : t(HEAR),
  }
}

/** The tooltip itself, drawn where the mouse rests on the name. */
export function ChannelNameTip({ tip, x, y }: { tip: ChannelTip; x: number; y: number }) {
  return createPortal(
    <div
      role="tooltip"
      data-testid="channel-name-tip"
      style={{
        position: 'fixed', left: x, top: y, zIndex: 10001, width: 300, pointerEvents: 'none',
        background: 'rgba(12,12,18,0.97)', border: '1px solid rgba(255,255,255,0.14)', borderRadius: 6,
        padding: '7px 9px', fontSize: 11, lineHeight: 1.45, color: '#e4e4e7',
        boxShadow: '0 10px 26px rgba(0,0,0,0.5)',
      }}
    >
      <b style={{ color: '#fff' }}>{tip.name}</b>: {tip.what}
      <br />
      {tip.how}
    </div>,
    document.body,
  )
}
