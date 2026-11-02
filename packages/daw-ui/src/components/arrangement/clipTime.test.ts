import { describe, expect, it } from 'vitest'
import { fileLayout, stretchLabel } from './clipTime'
import { PPQ, secsAtTick, tickAtSecs, type TempoPoint } from '../../utils/tempoTime'

const BAR = PPQ * 4
const steady: TempoPoint[] = [{ tick: 0, bpm: 150, ramp: 'instant' }]
const ramp: TempoPoint[] = [
  { tick: 0, bpm: 150, ramp: 'instant' },
  { tick: BAR * 8, bpm: 150, ramp: 'linear' },
  { tick: BAR * 16 - 1, bpm: 160, ramp: 'instant' },
  { tick: BAR * 16, bpm: 150, ramp: 'instant' },
]
const pxPerTick = 0.05

describe('where a clip draws its file', () => {
  it('lays a loop that follows the tempo over its own beats', () => {
    // A 150 loop: a beat of file (0.4 s) is a beat of playlist, ramp or not.
    const l = fileLayout({ entries: ramp, anchorTick: BAR * 4, lengthTicks: BAR * 12, pxPerTick, sourceStartSec: 0, timeFactor: 1, followBpm: 150 })
    expect(l.linear).toBe(true)
    expect(l.xAt(0.4 * 40)).toBeCloseTo(40 * PPQ * pxPerTick, 6)
  })

  it('draws a clip that keeps its speed at the tempo where it is', () => {
    const l = fileLayout({ entries: steady, anchorTick: 0, lengthTicks: BAR, pxPerTick, sourceStartSec: 1, timeFactor: 1 })
    expect(l.linear).toBe(true)
    // A second of the file after where the clip starts in it.
    expect(l.xAt(2)).toBeCloseTo((150 / 60) * PPQ * pxPerTick, 6)
    expect(l.secAt(l.xAt(2.5))).toBeCloseTo(2.5, 9)
  })

  it('squeezes a clip that keeps its speed where the song speeds up', () => {
    const at = BAR * 6
    const l = fileLayout({ entries: ramp, anchorTick: at, lengthTicks: BAR * 8, pxPerTick, sourceStartSec: 0, timeFactor: 1 })
    expect(l.linear).toBe(false)
    // Ten seconds in, the file is where ten seconds of the song have gone.
    const tick = tickAtSecs(ramp, secsAtTick(ramp, at) + 10)
    expect(l.xAt(10)).toBeCloseTo((tick - at) * pxPerTick, 6)
    expect(l.secAt(l.xAt(10))).toBeCloseTo(10, 6)
  })

  it('says how far a loop is stretched', () => {
    expect(stretchLabel(ramp, 0, BAR * 8, 150, 1)).toBeNull()
    expect(stretchLabel(ramp, 0, BAR * 16, 150, 1)).toBe('stretched 1.00 → 1.07')
    expect(stretchLabel(steady, 0, BAR, 128, 1)).toBe('stretched 1.17')
    expect(stretchLabel(steady, 0, BAR, null, 1)).toBeNull()
  })
})
