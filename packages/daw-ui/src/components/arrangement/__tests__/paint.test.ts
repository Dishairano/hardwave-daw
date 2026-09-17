import { describe, it, expect } from 'vitest'
import { paintSlotOccupied, paintSlotTick } from '../paint'
import type { ClipInfo } from '../../../stores/trackStore'

const PPQ = 960

function clip(position: number, length: number): ClipInfo {
  return {
    id: `c${position}`,
    name: 'clip',
    kind: 'audio',
    source_id: 's',
    position_ticks: position,
    length_ticks: length,
    muted: false,
    gainDb: 0,
    fadeInTicks: 0,
    fadeOutTicks: 0,
    reversed: false,
    pitchSemitones: 0,
  } as ClipInfo
}

describe('paintSlotTick', () => {
  it('paints the slot the pointer is inside, not the nearest line', () => {
    // Just past three quarters of a beat: the copy belongs in the beat the
    // pointer is in, not in the next one.
    const pixelsPerTick = 0.05
    const x = PPQ * 0.75 * pixelsPerTick
    expect(paintSlotTick(x, 0, pixelsPerTick, PPQ)).toBe(0)
  })

  it('advances one slot at a time across the grid', () => {
    const pixelsPerTick = 0.05
    const ticks = [0, 1.2, 2.9, 3].map(beat =>
      paintSlotTick(beat * PPQ * pixelsPerTick, 0, pixelsPerTick, PPQ),
    )
    expect(ticks).toEqual([0, PPQ, 2 * PPQ, 3 * PPQ])
  })

  it('counts the scrolled distance, so painting lands under the cursor', () => {
    const pixelsPerTick = 0.05
    const scroll = 4 * PPQ * pixelsPerTick
    expect(paintSlotTick(0, scroll, pixelsPerTick, PPQ)).toBe(4 * PPQ)
  })

  it('never paints before the start of the timeline', () => {
    expect(paintSlotTick(-500, 0, 0.05, PPQ)).toBe(0)
  })

  it('does not divide by a zero zoom or a zero step', () => {
    expect(paintSlotTick(100, 0, 0, PPQ)).toBe(0)
    expect(paintSlotTick(100, 0, 0.05, 0)).toBe(0)
  })
})

describe('paintSlotOccupied', () => {
  it('leaves an empty slot alone', () => {
    expect(paintSlotOccupied([], 0, PPQ)).toBe(false)
    expect(paintSlotOccupied(undefined, 0, PPQ)).toBe(false)
  })

  it('refuses to bury a clip that already sits there', () => {
    expect(paintSlotOccupied([clip(0, PPQ)], 0, PPQ)).toBe(true)
  })

  it('refuses a slot a longer clip only overlaps part of', () => {
    expect(paintSlotOccupied([clip(0, 4 * PPQ)], 2 * PPQ, PPQ)).toBe(true)
  })

  it('allows the slot that starts where another clip ends', () => {
    // Clips that touch do not overlap, so painting end to end must work.
    expect(paintSlotOccupied([clip(0, PPQ)], PPQ, PPQ)).toBe(false)
  })
})
