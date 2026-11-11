import { describe, expect, it } from 'vitest'
import {
  buildRowIndex, itemAt, itemsInBox, itemsInView, rowAtY, rowName, shownRowCount, silentRows, trackPlaces,
} from './rowIndex'
import sharedCasesText from './shownRows.cases.json?raw'

const clip = (id: string, at: number, len: number, row?: number | null) => ({
  id, position_ticks: at, length_ticks: len, row,
})
const auto = (id: string, at: number, len: number, row?: number | null) => ({
  id, startTick: at, lengthTicks: len, row,
})
const track = (
  id: string,
  clips: ReturnType<typeof clip>[] = [],
  kind = 'Audio',
  automationClips: ReturnType<typeof auto>[] = [],
  automationLanes: { id: string; row?: number | null }[] = [],
) => ({ id, kind, clips, automationClips, automationLanes })

describe('the row index', () => {
  it('puts clips of different channels on the rows they name, sorted by time', () => {
    const index = buildRowIndex([
      track('kick', [clip('k2', 960, 960, 3), clip('k1', 0, 960, 3)]),
      track('snare', [clip('s1', 480, 960, 3), clip('s2', 0, 960, 7)]),
    ])
    expect(index.byRow.get(3)!.map((i) => i.clip.id)).toEqual(['k1', 's1', 'k2'])
    expect(index.byRow.get(7)!.map((i) => i.track.id)).toEqual(['snare'])
    expect(index.used).toBe(8)
    expect(index.rowOf.get('s2')).toBe(7)
  })

  it('draws a clip with no row where its channel\'s other clips are', () => {
    const index = buildRowIndex([
      track('m', [], 'Master'),
      track('a', [clip('a1', 0, 960, 5), clip('a2', 960, 960, 5), clip('a3', 0, 960, 2), clip('new', 3840, 960)]),
    ])
    expect(index.rowOf.get('new')).toBe(5)
  })

  it('draws a clip with no row on its track\'s place when the track has no other row', () => {
    // A song built row by row: insert 1 is the first row, insert 3 the third.
    const index = buildRowIndex([
      track('master', [], 'Master'),
      track('insert-001', [clip('kick', 0, 960)]),
      track('insert-002'),
      track('insert-003', [clip('bass', 0, 960, null)]),
    ])
    expect(index.rowOf.get('kick')).toBe(0)
    expect(index.rowOf.get('bass')).toBe(2)
    expect(trackPlaces([{ kind: 'Master' }, { kind: 'Audio' }, { kind: 'Midi' }])).toEqual([-1, 0, 1])
  })

  it('gives automation with no row the first empty rows, one each', () => {
    const index = buildRowIndex([
      track('master', [], 'Master', [auto('vol', 0, 960), auto('pan', 0, 960)]),
      track('a', [clip('a1', 0, 960, 0), clip('a2', 0, 960, 2)], 'Audio', [auto('placed', 0, 960, 9)], [{ id: 'lane' }]),
    ])
    expect(index.rowOf.get('placed')).toBe(9)
    expect(index.rowOf.get('vol')).toBe(1)
    expect(index.rowOf.get('pan')).toBe(3)
    expect(index.rowOf.get('lane')).toBe(4)
    expect(index.lanes.get(4)![0].lane.id).toBe('lane')
    expect(index.byRow.get(1)![0].kind).toBe('auto')
  })

  it('finds only what reaches into the view, long clips that started before it included', () => {
    const clips = [clip('long', 0, 100_000, 0)]
    for (let i = 0; i < 472; i++) clips.push(clip(`c${i}`, 1000 + i * 960, 480, 0))
    const index = buildRowIndex([track('a', clips)])
    const seen = itemsInView(index, 0, 50_000, 52_000).map((i) => i.clip.id)
    expect(seen).toEqual(['long', 'c51', 'c52', 'c53'])
    expect(itemsInView(index, 1, 0, 1e9)).toEqual([])
  })

  it('hit-tests the clip drawn on top, with a little slack at the edges', () => {
    const index = buildRowIndex([
      track('a', [clip('under', 0, 2000, 4)]),
      track('b', [clip('over', 1000, 2000, 4)]),
    ])
    expect(itemAt(index, 4, 1500)?.clip.id).toBe('over')
    expect(itemAt(index, 4, 500)?.clip.id).toBe('under')
    expect(itemAt(index, 4, 3010)).toBeUndefined()
    expect(itemAt(index, 4, 3010, 20)?.clip.id).toBe('over')
    expect(itemAt(index, 5, 1500)).toBeUndefined()
  })

  it('picks what a rubber band covers across rows', () => {
    const index = buildRowIndex([
      track('a', [clip('a', 0, 960, 0), clip('b', 0, 960, 1), clip('c', 5000, 960, 1), clip('d', 0, 960, 3)]),
    ])
    expect(itemsInBox(index, 0, 1, 0, 1000).map((i) => i.clip.id)).toEqual(['a', 'b'])
  })
})

describe('rows on screen', () => {
  it('finds the row under the pointer, scrolled or not', () => {
    expect(rowAtY(10, 22, 40, 0)).toBe(-1)
    expect(rowAtY(22, 22, 40, 0)).toBe(0)
    expect(rowAtY(70, 22, 40, 0)).toBe(1)
    expect(rowAtY(70, 22, 40, 400)).toBe(11)
  })

  it('shows FL\'s 500 rows, more when the song uses more', () => {
    expect(shownRowCount({ used: 22 }, 0)).toBe(500)
    expect(shownRowCount({ used: 640 }, 501)).toBe(640)
    expect(shownRowCount({ used: 3 }, 700)).toBe(700)
  })

  it('names an unnamed row "Track N", as FL does', () => {
    expect(rowName([{ name: 'Drums' }, { name: '' }], 0)).toBe('Drums')
    expect(rowName([{ name: 'Drums' }, { name: '' }], 1)).toBe('Track 2')
    expect(rowName([], 21)).toBe('Track 22')
  })

  it('silences muted rows, and every other row while one is soloed', () => {
    const none = silentRows([{ muted: false, soloed: false }])
    expect(none(0) || none(9)).toBe(false)
    const muted = silentRows([{ muted: false, soloed: false }, { muted: true, soloed: false }])
    expect([muted(0), muted(1), muted(5)]).toEqual([false, true, false])
    const solo = silentRows([{ muted: false, soloed: true }, { muted: false, soloed: false }, { muted: true, soloed: true }])
    expect([solo(0), solo(1), solo(2), solo(9)]).toEqual([false, true, true, true])
  })
})

describe('the rule shared with the engine', () => {
  // The same cases crates/hardwave-project/src/layout.rs checks
  // `shown_rows` on: a row's mute and solo silence exactly what the
  // playlist draws on it.
  const cases = JSON.parse(sharedCasesText) as {
    tracks: {
      id: string
      kind: string
      clips: { id: string; row: number | null }[]
      automationClips: { id: string; row: number | null }[]
      automationLanes: { id: string; row: number | null }[]
    }[]
    shown: Record<string, number>
  }

  it('draws what has no row of its own where the engine says it is', () => {
    const tracks = cases.tracks.map((t) => ({
      ...t,
      clips: t.clips.map((c) => clip(c.id, 0, 960, c.row)),
      automationClips: t.automationClips.map((c) => auto(c.id, 0, 960, c.row)),
    }))
    const index = buildRowIndex(tracks)
    expect(Object.fromEntries(index.rowOf)).toEqual(cases.shown)
  })
})
