import { describe, expect, it } from 'vitest'
import { playlistColumnRows, playlistRowLayout, playlistTracks, visibleRows } from './playlistRows'

const track = (id: string, lanes = 0, clips = 0, kind = 'Audio') => ({
  id,
  kind,
  automationLanes: Array.from({ length: lanes }),
  automationClips: Array.from({ length: clips }),
})

describe('playlist rows', () => {
  it('puts automation rows under their track and pushes the rest down', () => {
    const layout = playlistRowLayout([track('a'), track('b', 1, 1), track('c')])
    expect(layout.rowOf).toEqual([0, 1, 4])
    expect(layout.ownerOf).toEqual([0, 1, 1, 1, 2])
    expect(layout.isTrackRow).toEqual([true, true, false, false, true])
    expect(layout.total).toBe(5)
  })

  it('shows every track but the master and collapsed folders', () => {
    const tracks = [track('m', 0, 0, 'Master'), track('insert-001'), track('channel'), track('hidden')]
    const shown = playlistTracks(tracks, [{ collapsed: true, trackIds: ['hidden'] }])
    expect(shown.map((t) => t.id)).toEqual(['insert-001', 'channel'])
  })
})

describe('the name column draws what is in view', () => {
  const t = (id: string, lanes = 0) => ({ id, kind: 'Audio', automationLanes: Array.from({ length: lanes }, (_, i) => ({ id: `${id}-l${i}` })) })

  it('lists tracks, their lanes, then the empty slots', () => {
    const rows = playlistColumnRows([t('a', 1), t('b')], 2)
    expect(rows.map(r => r.kind)).toEqual(['track', 'lane', 'track', 'empty', 'empty'])
    expect(rows[4]).toEqual({ kind: 'empty', slot: 4 })
  })

  it('windows the rows around the scroll position', () => {
    expect(visibleRows(500, 56, 0, 560)).toEqual({ first: 0, last: 14 })
    expect(visibleRows(500, 56, 5600, 560)).toEqual({ first: 96, last: 114 })
    expect(visibleRows(500, 56, 56 * 499, 560)).toEqual({ first: 495, last: 500 })
    expect(visibleRows(3, 56, 0, 560)).toEqual({ first: 0, last: 3 })
  })
})
