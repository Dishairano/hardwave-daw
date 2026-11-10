import { describe, expect, it } from 'vitest'
import { visibleRows } from './playlistRows'

describe('the name column draws what is in view', () => {
  it('windows the rows around the scroll position', () => {
    expect(visibleRows(500, 56, 0, 560)).toEqual({ first: 0, last: 14 })
    expect(visibleRows(500, 56, 5600, 560)).toEqual({ first: 96, last: 114 })
    expect(visibleRows(500, 56, 56 * 499, 560)).toEqual({ first: 495, last: 500 })
    expect(visibleRows(3, 56, 0, 560)).toEqual({ first: 0, last: 3 })
  })
})
