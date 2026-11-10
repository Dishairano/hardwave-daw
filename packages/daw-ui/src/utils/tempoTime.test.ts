import { describe, expect, it } from 'vitest'
import { PPQ, averageBpmBetween, bpmAtTick, layoutBpm, sampleAtTick, secsAtTick, steadyBpmBetween, tickAtSample, tickAtSecs, type TempoPoint } from './tempoTime'

const BAR = PPQ * 4
/** 150, a ramp up to 160 over bars 9 to 17, and back to 150 for the drop:
 *  the song the tempo lane was drawn for. */
const buildAndDrop: TempoPoint[] = [
  { tick: 0, bpm: 150, ramp: 'instant' },
  { tick: BAR * 8, bpm: 150, ramp: 'linear' },
  { tick: BAR * 16 - 1, bpm: 160, ramp: 'instant' },
  { tick: BAR * 16, bpm: 150, ramp: 'instant' },
]

describe('ticks and seconds through the tempo map', () => {
  it('counts a steady stretch at its tempo', () => {
    expect(secsAtTick(buildAndDrop, BAR * 8)).toBeCloseTo(12.8, 9)
    expect(tickAtSecs(buildAndDrop, 12.8)).toBeCloseTo(BAR * 8, 6)
  })

  it('integrates a ramp the way the engine does', () => {
    const build = (32 * 60 * Math.log(160 / 150)) / 10
    expect(secsAtTick(buildAndDrop, BAR * 16)).toBeCloseTo(12.8 + build, 3)
    expect(bpmAtTick(buildAndDrop, BAR * 12)).toBeCloseTo(155, 2)
    expect(bpmAtTick(buildAndDrop, BAR * 16)).toBe(150)
  })

  it('comes back to the tick it started from, everywhere in the song', () => {
    for (let tick = 0; tick < BAR * 24; tick += 233) {
      expect(tickAtSecs(buildAndDrop, secsAtTick(buildAndDrop, tick))).toBeCloseTo(tick, 4)
    }
    expect(tickAtSample(buildAndDrop, sampleAtTick(buildAndDrop, BAR * 20, 48000), 48000)).toBeCloseTo(BAR * 20, 0)
  })

  it('lays the playlist out at the opening tempo', () => {
    expect(layoutBpm(buildAndDrop)).toBe(150)
    expect(layoutBpm([])).toBe(140)
  })

  it('knows where the tempo holds still', () => {
    expect(steadyBpmBetween(buildAndDrop, 0, BAR * 8)).toBe(150)
    expect(steadyBpmBetween(buildAndDrop, BAR * 7, BAR * 9)).toBeNull()
    expect(steadyBpmBetween(buildAndDrop, BAR * 16, BAR * 20)).toBe(150)
  })

  it('treats an empty map as the default song', () => {
    expect(secsAtTick([], PPQ)).toBeCloseTo(60 / 140, 9)
  })

  it('averages the tempo over a stretch the way the engine does', () => {
    expect(averageBpmBetween(buildAndDrop, 0, BAR * 8)).toBeCloseTo(150, 9)
    const avg = averageBpmBetween(buildAndDrop, BAR * 8, BAR * 16)
    expect(avg).toBeGreaterThan(150)
    expect(avg).toBeLessThan(160)
  })
})
