import { describe, expect, it } from 'vitest'
import { inRackGroup, keptRackGroupFilter, rackGroupButtons } from './rackGroupFilter'

/** The founder's FL song: 92 audio clip channels, 9 automation, 5 unsorted. */
const fromFl = [
  ...Array.from({ length: 5 }, () => ({ rackGroup: 2 })),
  ...Array.from({ length: 92 }, () => ({ rackGroup: 0 })),
  ...Array.from({ length: 9 }, () => ({ rackGroup: 1 })),
]

describe("the channel rack's group filter", () => {
  it("is All, then the song's own groups with their counts", () => {
    const buttons = rackGroupButtons(['Audio', 'Automation', 'Unsorted'], fromFl)
    expect(buttons.map((b) => [b.label, b.count])).toEqual([
      ['All', 106],
      ['Audio', 92],
      ['Automation', 9],
      ['Unsorted', 5],
    ])
    expect(buttons.map((b) => b.filter)).toEqual(['all', 0, 1, 2])
  })

  it('a song made here has its one group', () => {
    const buttons = rackGroupButtons(['Unsorted'], [{ rackGroup: 0 }, { rackGroup: 0 }])
    expect(buttons.map((b) => [b.label, b.count])).toEqual([['All', 2], ['Unsorted', 2]])
  })

  it('shows a group with no channels, and counts no channel twice', () => {
    const buttons = rackGroupButtons(['Drums', 'Leads'], [{ rackGroup: 0 }, { rackGroup: null }, { rackGroup: 7 }])
    expect(buttons.map((b) => b.count)).toEqual([3, 1, 0])
  })

  it('All shows every channel, a group only its own', () => {
    expect(inRackGroup('all', 2)).toBe(true)
    expect(inRackGroup('all', null)).toBe(true)
    expect(inRackGroup(1, 1)).toBe(true)
    expect(inRackGroup(1, 0)).toBe(false)
    expect(inRackGroup(0, null)).toBe(false)
    expect(fromFl.filter((c) => inRackGroup(1, c.rackGroup))).toHaveLength(9)
  })

  it('a group the next song does not have falls back to All', () => {
    expect(keptRackGroupFilter(2, ['Audio', 'Automation', 'Unsorted'])).toBe(2)
    expect(keptRackGroupFilter(2, ['Unsorted'])).toBe('all')
    expect(keptRackGroupFilter('all', [])).toBe('all')
  })
})
