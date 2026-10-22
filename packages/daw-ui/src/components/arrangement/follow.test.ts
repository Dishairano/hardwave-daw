import { describe, expect, it } from 'vitest'
import { followScroll } from './follow'

describe('following the playhead', () => {
  it('keeps the playhead a quarter in when scrolling smoothly', () => {
    expect(followScroll(1000, 800, 'continuous')).toBe(800)
    expect(followScroll(100, 800, 'continuous')).toBe(0)
  })

  it('turns pages: the view holds still, then jumps a page', () => {
    expect(followScroll(100, 1000, 'page')).toBe(0)
    expect(followScroll(899, 1000, 'page')).toBe(0)
    expect(followScroll(900, 1000, 'page')).toBe(900)
    expect(followScroll(1799, 1000, 'page')).toBe(900)
    expect(followScroll(1800, 1000, 'page')).toBe(1800)
  })
})
