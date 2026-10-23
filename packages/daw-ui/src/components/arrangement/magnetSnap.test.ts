import { describe, expect, it } from 'vitest'
import { magnetEdge, magnetSnap } from './magnetSnap'

describe('magnetic snap', () => {
  it('sticks a clip start to a nearby edge, off the grid', () => {
    // A take ends at 1003; the grid would put the clip at 960.
    expect(magnetSnap(1010, 480, 960, [1003], 20)).toBe(1003)
  })

  it('sticks a clip end to a nearby edge', () => {
    // A 480-tick clip dragged so it ends near a marker at 2000.
    expect(magnetSnap(1515, 480, 1440, [2000], 20)).toBe(1520)
  })

  it('leaves it to the grid when nothing is within reach', () => {
    expect(magnetSnap(1200, 480, 1440, [3000], 20)).toBe(1440)
  })

  it('takes the nearest of several and never goes before the song start', () => {
    expect(magnetSnap(105, 200, 0, [100, 110, 150], 20)).toBe(100)
    expect(magnetSnap(5, 200, 0, [190], 20)).toBe(0)
  })

  it('works for one edge being resized', () => {
    expect(magnetEdge(2992, 2880, [3000], 20)).toBe(3000)
  })
})
