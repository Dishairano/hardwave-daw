import { describe, expect, it } from 'vitest'
import { zoomStep } from './zoom'

const NOTCH = -120 // one mouse-wheel notch towards you: zoom in

function notches(from: number, to: number): number {
  let z = from
  let n = 0
  while (z < to && n < 500) { z *= zoomStep(NOTCH, z); n++ }
  return n
}

describe('playlist wheel zoom', () => {
  it('keeps a notch at about 18 % up to the old top of the range', () => {
    expect(zoomStep(NOTCH, 1)).toBeCloseTo(1.183, 2)
    expect(zoomStep(NOTCH, 16)).toBeCloseTo(1.183, 2)
  })

  it('reaches the samples in about a dozen notches past the old top', () => {
    expect(notches(16, 4000)).toBeLessThanOrEqual(14)
    expect(notches(16, 4000)).toBeGreaterThanOrEqual(9)
  })

  it('zooms out by the same steps it zooms in', () => {
    const z = 600
    expect(z * zoomStep(NOTCH, z) * zoomStep(-NOTCH, z)).toBeCloseTo(z, 6)
  })
})
