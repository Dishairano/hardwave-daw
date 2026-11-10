import { describe, it, expect } from 'vitest'
import type { TrackInfo } from '../stores/trackStore'
import {
  FL_INSERT_COUNT,
  insertBoxText,
  insertName,
  insertOf,
  lastMixerNumber,
  mixerSlots,
  mixerSlotsKey,
  parseInsertNumber,
  rackChannels,
  stepNumber,
} from './mixerNumbers'

const tr = (id: string, kind: string, mixerNumber: number | null, more: Partial<TrackInfo> = {}) =>
  ({ id, name: id, kind, mixerNumber, rackGroup: null, outputBus: null, ...more } as unknown as TrackInfo)

/** A song made before FL's three lists, as step 1 numbers it: the master
 *  0, then every track that is not automation 1, 2, 3 in list order. */
function songMadeBefore(): TrackInfo[] {
  const tracks = [tr('master', 'Master', 0)]
  for (let i = 1; i <= 500; i++) tracks.push(tr(`insert-${String(i).padStart(3, '0')}`, 'Audio', i))
  tracks.push(tr('lead', 'Midi', 501, { rackGroup: 0 }))
  tracks.push(tr('auto', 'Automation', null, { rackGroup: 0 }))
  return tracks
}

/** FL's shape: the master, inserts 1 to 20 (9 to 16 named, nothing in them),
 *  channels without an insert of their own routed to one. */
function flSong(): TrackInfo[] {
  const tracks = [tr('master', 'Master', 0)]
  for (let n = 1; n <= 20; n++) tracks.push(tr(`ins-${n}`, 'Bus', n, { name: n === 7 ? 'THM - Pad 11' : '' }))
  tracks.push(tr('kick', 'Audio', null, { rackGroup: 0, outputBus: 'ins-4' }))
  tracks.push(tr('hat', 'Audio', null, { rackGroup: 0 }))
  tracks.push(tr('vol', 'Automation', null, { rackGroup: 1 }))
  return tracks
}

describe('the mixer by number', () => {
  it('a song made before shows the same strips in the same order, 1 to 501, none empty', () => {
    const slots = mixerSlots(songMadeBefore())
    expect(slots).toHaveLength(501)
    expect(slots.every((s, i) => s.number === i + 1 && s.trackId !== null)).toBe(true)
    expect(slots[0].trackId).toBe('insert-001')
    expect(slots[500].trackId).toBe('lead')
  })

  it("FL's inserts by number, then the empty numbers up to 125; channels are not strips", () => {
    const slots = mixerSlots(flSong())
    expect(slots).toHaveLength(FL_INSERT_COUNT)
    expect(slots[6]).toEqual({ number: 7, trackId: 'ins-7' })
    expect(slots.slice(20).every((s) => s.trackId === null)).toBe(true)
    expect(slots.some((s) => s.trackId === 'kick' || s.trackId === 'vol')).toBe(false)
  })

  it('numbers past 125 show up to the highest one used, with the gaps empty', () => {
    const tracks = [...flSong(), tr('far', 'Bus', 140)]
    expect(lastMixerNumber(tracks)).toBe(140)
    const slots = mixerSlots(tracks)
    expect(slots).toHaveLength(140)
    expect(slots[139].trackId).toBe('far')
    expect(slots[130].trackId).toBeNull()
  })

  it('an insert nobody named reads "Insert N"', () => {
    expect(insertName(4, { name: '' })).toBe('Insert 4')
    expect(insertName(7, { name: 'THM - Pad 11' })).toBe('THM - Pad 11')
    expect(insertName(25)).toBe('Insert 25')
    expect(insertName(0, { name: 'Out' })).toBe('Master')
  })

  it('the row key changes when a number comes or goes, not when a fader moves', () => {
    const tracks = flSong()
    const key = mixerSlotsKey(tracks)
    const moved = tracks.map((t) => (t.id === 'ins-3' ? { ...t, volume_db: -6 } : t))
    expect(mixerSlotsKey(moved)).toBe(key)
    expect(mixerSlotsKey([...tracks, tr('new', 'Bus', 21)])).not.toBe(key)
  })
})

describe('the insert box', () => {
  const tracks = flSong()
  const byId = Object.fromEntries(tracks.map((t) => [t.id, t]))

  it('a channel routed to an insert shows its number, one routed nowhere M', () => {
    expect(insertOf(byId.kick, byId)).toBe(4)
    expect(insertOf(byId.hat, byId)).toBe(0)
    expect(insertBoxText(insertOf(byId.hat, byId))).toBe('M')
  })

  it('a channel with an insert of its own shows it until it is routed elsewhere', () => {
    const own = tr('lead', 'Midi', 21, { rackGroup: 0 })
    expect(insertOf(own, byId)).toBe(21)
    expect(insertOf({ ...own, outputBus: 'ins-7' }, byId)).toBe(7)
    expect(insertOf({ ...own, outputBus: 'master' }, { ...byId, master: tr('master', 'Master', 0) })).toBe(0)
  })

  it('an automation channel has no insert', () => {
    expect(insertOf(byId.vol, byId)).toBeNull()
    expect(insertBoxText(null)).toBe('–')
  })

  it('drag and scroll step between M and the last number; typing takes M or a number', () => {
    expect(stepNumber(0, -1, 125)).toBe(0)
    expect(stepNumber(124, 3, 125)).toBe(125)
    expect(stepNumber(4, 2, 125)).toBe(6)
    expect(parseInsertNumber('m', 125)).toBe(0)
    expect(parseInsertNumber(' 17 ', 125)).toBe(17)
    expect(parseInsertNumber('126', 125)).toBeNull()
    expect(parseInsertNumber('4a', 125)).toBeNull()
  })
})

describe('the channel rack', () => {
  it('lists the tracks in a rack group, in song order, whatever their ids', () => {
    expect(rackChannels(flSong()).map((t) => t.id)).toEqual(['kick', 'hat', 'vol'])
    // A song made before: the instrument after the 500 rows and the
    // automation track; the numbered inserts are not channels.
    expect(rackChannels(songMadeBefore()).map((t) => t.id)).toEqual(['lead', 'auto'])
  })
})
