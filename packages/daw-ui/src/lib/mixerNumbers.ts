import type { TrackInfo } from '../stores/trackStore'

/**
 * One numbering for the mixer, the channel rack and the sample window, as
 * FL has: the master is 0 ("M"), the inserts 1, 2, 3 and on, each a track
 * with that `mixerNumber`. A number no track has is an empty insert: the
 * mixer draws it dimmed, and it becomes a track when something is routed
 * to it or put on it. Before this the mixer counted strips by their place
 * in the track list, the rack counted the same way, and the sample window
 * read the number out of an "insert-007" id, so the three disagreed.
 */

/** FL's mixer has 125 inserts; the mixer draws every number up to here, or
 *  up to the highest one in use when a song has more. */
export const FL_INSERT_COUNT = 125

/** One strip after the master: a number, and the track that is that insert
 *  or null when the number is empty. */
export interface MixerSlot {
  number: number
  trackId: string | null
}

type Numbered = Pick<TrackInfo, 'id' | 'kind' | 'mixerNumber'>

/** The inserts by number. The master is left out: it has its own strip. */
export function insertsByNumber<T extends Numbered>(tracks: T[]): Map<number, T> {
  const byNumber = new Map<number, T>()
  for (const t of tracks) {
    const n = t.mixerNumber
    if (t.kind === 'Master' || n == null || n < 1 || byNumber.has(n)) continue
    byNumber.set(n, t)
  }
  return byNumber
}

/** The highest number the mixer shows: 125, or more when a song uses more. */
export function lastMixerNumber(tracks: Numbered[]): number {
  let highest = 0
  for (const n of insertsByNumber(tracks).keys()) highest = Math.max(highest, n)
  return Math.max(FL_INSERT_COUNT, highest)
}

/** The mixer's strips after the master, one per number from 1 to the last,
 *  with the empty numbers in their places. A channel without an insert
 *  (no number) is not a strip. */
export function mixerSlots(tracks: Numbered[]): MixerSlot[] {
  const byNumber = insertsByNumber(tracks)
  const last = lastMixerNumber(tracks)
  const slots: MixerSlot[] = new Array(last)
  for (let n = 1; n <= last; n++) slots[n - 1] = { number: n, trackId: byNumber.get(n)?.id ?? null }
  return slots
}

/** What a strip or a list says for an insert: its name, or "Insert N" as FL
 *  calls one nobody named. The master is "Master". */
export function insertName(number: number, track?: Pick<TrackInfo, 'name'> | null): string {
  if (number === 0) return 'Master'
  const name = track?.name?.trim()
  return name ? name : `Insert ${number}`
}

/** The insert box's text: M for the master, the number, or a dash when
 *  there is none. */
export function insertBoxText(number: number | null): string {
  return number === 0 ? 'M' : number == null ? '–' : String(number)
}

/**
 * The insert a channel plays into, as FL's insert box shows it: the number
 * of the insert it is routed to (0 for the master), or, routed nowhere, its
 * own insert when it has one, else the master. An automation channel makes
 * no sound and has none (null), and so has a track routed to something
 * that is not in the mixer.
 */
export function insertOf(
  track: Pick<TrackInfo, 'kind' | 'mixerNumber' | 'outputBus'>,
  byId: Record<string, Pick<TrackInfo, 'kind' | 'mixerNumber'> | undefined>,
): number | null {
  if (track.kind === 'Automation') return null
  if (track.kind === 'Master') return 0
  if (track.outputBus) {
    const target = byId[track.outputBus]
    if (!target) return track.mixerNumber ?? 0
    return target.kind === 'Master' ? 0 : (target.mixerNumber ?? null)
  }
  return track.mixerNumber ?? 0
}

/** The channels the rack lists, in rack order (the song's track order):
 *  every track in a rack group. */
export function rackChannels<T extends Pick<TrackInfo, 'kind' | 'rackGroup'>>(tracks: T[]): T[] {
  return tracks.filter((t) => t.kind !== 'Master' && t.rackGroup != null)
}

/** A short key that only changes when the mixer's strips do, so the mixer
 *  re-lays its row when a number comes or goes, not on every fader move. */
export function mixerSlotsKey(tracks: Numbered[]): string {
  let key = ''
  for (const t of tracks) {
    if (t.kind === 'Master' || t.mixerNumber == null) continue
    key += `${t.mixerNumber}:${t.id};`
  }
  return key
}

/** The number a step up or down from `from` lands on, for dragging or
 *  scrolling on the insert box: 0 (the master) to the mixer's last. */
export function stepNumber(from: number, by: number, last: number): number {
  return Math.max(0, Math.min(last, from + by))
}

/** What a typed number means in the insert list: "M" or 0 the master, a
 *  whole number up to the last, or null for anything else. */
export function parseInsertNumber(text: string, last: number): number | null {
  const s = text.trim()
  if (/^m$/i.test(s)) return 0
  if (!/^\d+$/.test(s)) return null
  const n = Number(s)
  return n <= last ? n : null
}
