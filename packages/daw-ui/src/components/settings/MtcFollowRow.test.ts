import { describe, it, expect } from 'vitest'
import { formatTimecode, parseStart } from './MtcFollowRow'

describe('timecode text', () => {
  it('formats seconds at the master frame rate', () => {
    expect(formatTimecode(3612 + 5 / 25, 25)).toBe('01:00:12:05')
    expect(formatTimecode(0, 30)).toBe('00:00:00:00')
  })

  it('reads a start time and refuses what is not one', () => {
    expect(parseStart('01:00:00')).toBe(3600)
    expect(parseStart(' 0:01:30 ')).toBe(90)
    expect(parseStart('01:60:00')).toBeNull()
    expect(parseStart('1h')).toBeNull()
  })
})
