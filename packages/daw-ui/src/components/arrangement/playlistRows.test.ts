import { describe, expect, it } from 'vitest'
import { playlistRowLayout, playlistTracks } from './playlistRows'

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
