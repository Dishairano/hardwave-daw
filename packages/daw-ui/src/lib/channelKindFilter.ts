/** Which channels the All / Audio / MIDI buttons over the channel rack show. */
export type ChannelKindFilter = 'All' | 'Audio' | 'MIDI'

/**
 * Whether a channel of this track kind shows under the filter: sample
 * channels are Audio, instrument channels MIDI, and All shows everything.
 * The three buttons were drawn with All lit for good and did nothing.
 */
export function channelKindShown(filter: ChannelKindFilter, kind: string | undefined): boolean {
  if (filter === 'All') return true
  const k = (kind || '').toLowerCase()
  return filter === 'MIDI' ? k === 'midi' : k === 'audio'
}
