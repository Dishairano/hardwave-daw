import { describe, expect, it } from 'vitest'
import { renderBounds } from './exportBounds'

const loop = { start: 48_000, end: 96_000 }

describe('renderBounds', () => {
  it('asks for the pattern, not the first bars of the song', () => {
    expect(renderBounds('pattern', loop, 48_000, 48_000)).toEqual({
      startSamples: null,
      endSamples: null,
      pattern: true,
    })
  })

  it('renders the loop at the export rate', () => {
    expect(renderBounds('loop', loop, 48_000, 48_000)).toEqual({
      startSamples: 48_000,
      endSamples: 96_000,
      pattern: false,
    })
    expect(renderBounds('loop', loop, 48_000, 96_000)).toEqual({
      startSamples: 96_000,
      endSamples: 192_000,
      pattern: false,
    })
  })

  it('renders the whole song for the song, and for a loop with no length', () => {
    const none = { startSamples: null, endSamples: null, pattern: false }
    expect(renderBounds('full', loop, 48_000, 48_000)).toEqual(none)
    expect(renderBounds('loop', { start: 5, end: 5 }, 48_000, 48_000)).toEqual(none)
  })
})
