import { describe, expect, it } from 'vitest'
import { barLabel, bpmToY, changeAt, clampTick, guideValues, laneScale, pointAt, roundBpm, tempoSummary, yToBpm } from './tempoLane'
import type { TempoPoint } from '../../utils/tempoTime'
import { meterSegments } from '../../utils/meter'

const BAR = 3840
const song: TempoPoint[] = [
  { tick: 0, bpm: 150, ramp: 'instant' },
  { tick: BAR * 8, bpm: 150, ramp: 'linear' },
  { tick: BAR * 16 - 1, bpm: 160, ramp: 'instant' },
  { tick: BAR * 16, bpm: 150, ramp: 'instant' },
]

describe('the tempo lane', () => {
  it('scales to the song, with room around it', () => {
    const s = laneScale(song)
    expect(s.lo).toBeLessThan(150)
    expect(s.hi).toBeGreaterThan(160)
    // A tempo goes to a height and back.
    expect(yToBpm(bpmToY(155, s), s)).toBeCloseTo(155, 9)
    // Faster is higher.
    expect(bpmToY(160, s)).toBeLessThan(bpmToY(150, s))
    expect(guideValues(s).length).toBeGreaterThan(0)
  })

  it('finds the point under the pointer, the later of two on one spot', () => {
    const s = laneScale(song)
    const x = (tick: number) => tick * 0.0125
    // The ramp's end (160) and the drop (150) are at one bar line.
    expect(pointAt(song, x(BAR * 16), bpmToY(160, s), x, s)).toBe(2)
    expect(pointAt(song, x(BAR * 16), bpmToY(150, s), x, s)).toBe(3)
    expect(pointAt(song, x(BAR * 4), bpmToY(150, s), x, s)).toBeNull()
  })

  it('keeps a moved point between its neighbours, and the first at the start', () => {
    expect(clampTick(song, 1, BAR * 20)).toBe(BAR * 16 - 2)
    expect(clampTick(song, 1, -50)).toBe(1)
    expect(clampTick(song, 0, BAR)).toBe(0)
    expect(clampTick(song, 3, BAR * 40)).toBe(BAR * 40)
  })

  it('knows which change is in force where', () => {
    expect(changeAt(song, BAR * 3)).toBe(0)
    expect(changeAt(song, BAR * 10)).toBe(1)
    expect(changeAt(song, BAR * 16)).toBe(3)
  })

  it('names the song the way the mockup does', () => {
    expect(tempoSummary(song)).toBe('150 → 160 → 150')
    expect(tempoSummary([{ tick: 0, bpm: 140, ramp: 'instant' }])).toBe('140')
    expect(roundBpm(160.04)).toBe(160)
    expect(roundBpm(5)).toBe(20)
  })

  it('names a spot by its bar, and the end of a ramp a tick early by the bar of the jump', () => {
    const segs = meterSegments([{ tick: 0, timeSigNum: 4, timeSigDen: 4 }])
    expect(barLabel(segs, BAR * 3)).toBe('bar 4')
    expect(barLabel(segs, BAR * 16 - 1)).toBe('bar 17')
    expect(barLabel(segs, BAR * 3 + 960 * 2)).toBe('bar 4 : 3')
  })
})
