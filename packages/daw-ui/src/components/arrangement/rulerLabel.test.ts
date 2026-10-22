import { describe, expect, it } from 'vitest'
import { rulerLabel } from './rulerLabel'

// At 120 BPM a bar of 4/4 is 2 seconds, 3840 ticks.
const BAR = 3840

describe('the ruler', () => {
  it('counts bars, time, timecode and samples from the same line', () => {
    expect(rulerLabel(BAR * 2, 3, 'bars', 120, 25, 48000)).toBe('3')
    expect(rulerLabel(BAR * 2, 3, 'time', 120, 25, 48000)).toBe('0:04.0')
    expect(rulerLabel(BAR * 2, 3, 'samples', 120, 25, 48000)).toBe('192000')
    expect(rulerLabel(BAR * 31, 32, 'timecode', 120, 25, 48000)).toBe('01:02:00')
  })

  it('counts frames at the chosen rate, and never shows a frame equal to the rate', () => {
    // 1.98 s lies in frame 23 of second 1 at 24 fps, and 1.999 s in the
    // last frame of second 1 at 30 fps: never frame 24 or 30.
    expect(rulerLabel((1.98 / 2) * BAR, 1, 'timecode', 120, 24, 48000)).toBe('00:01:23')
    expect(rulerLabel((1.999 / 2) * BAR, 1, 'timecode', 120, 30, 48000)).toBe('00:01:29')
    expect(rulerLabel(BAR, 2, 'timecode', 120, 30, 48000)).toBe('00:02:00')
  })
})
