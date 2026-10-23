import { describe, expect, it } from 'vitest'
import { rippleShifts } from './ripple'

const clip = (id: string, at: number, len: number) => ({ id, position_ticks: at, length_ticks: len })

describe('delete and close the gap', () => {
  it('moves what comes after a deleted clip back by its length', () => {
    const clips = [clip('a', 0, 100), clip('b', 100, 50), clip('c', 150, 100)]
    expect(rippleShifts(clips, new Set(['b']))).toEqual(new Map([['c', 100]]))
  })

  it('adds up several gaps and merges deleted clips that touch', () => {
    const clips = [clip('a', 0, 100), clip('b', 100, 50), clip('c', 150, 50), clip('d', 300, 10), clip('e', 400, 10)]
    // b and c touch (one 100-tick gap), d is a 10-tick gap further on.
    const moved = rippleShifts(clips, new Set(['b', 'c', 'd']))
    expect(moved.get('e')).toBe(400 - 100 - 10)
    expect(moved.has('a')).toBe(false)
  })

  it('leaves a space alone when a remaining clip still covers part of it', () => {
    const clips = [clip('a', 0, 120), clip('b', 100, 50), clip('c', 200, 50)]
    // a reaches into b's range: closing it would pile c onto a.
    expect(rippleShifts(clips, new Set(['b'])).size).toBe(0)
  })
})
