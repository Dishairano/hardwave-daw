import { describe, it, expect } from 'vitest'
import { sectionAt, sectionMoves } from './sectionMoves'

const song = [
  { id: 'c', name: 'Chorus', startTicks: 1920, endTicks: 3840 },
  { id: 'v', name: 'Verse', startTicks: 0, endTicks: 1920 },
  { id: 'd', name: 'Drop', startTicks: 3840, endTicks: 5760 },
]

describe('sectionAt', () => {
  it('finds the section under the tick, end excluded', () => {
    expect(sectionAt(song, 0)?.id).toBe('v')
    expect(sectionAt(song, 1920)?.id).toBe('c')
    expect(sectionAt(song, 5760)).toBeNull()
  })
})

describe('sectionMoves', () => {
  it('swaps with the neighbour on either side', () => {
    expect(sectionMoves(song, 'c')).toEqual({ earlier: 'v', later: null })
    expect(sectionMoves(song, 'v')).toEqual({ earlier: undefined, later: 'd' })
    expect(sectionMoves(song, 'd')).toEqual({ earlier: 'c', later: undefined })
  })

  it('has nothing to offer for an unknown section', () => {
    expect(sectionMoves(song, 'x')).toEqual({ earlier: undefined, later: undefined })
  })
})
