import { describe, expect, it } from 'vitest'
import { barStartAt, bpmAt, flagEntry, tempoFlags, type TempoMapEntry } from './tempoFlags'
import { meterSegments } from '../../utils/meter'

const entry = (tick: number, bpm: number, num = 4, den = 4): TempoMapEntry =>
  ({ tick, bpm, timeSigNum: num, timeSigDen: den, ramp: 'instant' })

describe('tempo flags', () => {
  it('flags each tempo change and each signature change, not the opening', () => {
    const flags = tempoFlags([entry(0, 140), entry(3840, 150), entry(7680, 150, 7, 8)])
    expect(flags.map((f) => [f.id, f.tick, f.label])).toEqual([
      ['tempo:1', 3840, '150 BPM'],
      ['timesig:2', 7680, '7/8'],
    ])
  })

  it('reads an entry back from a flag id', () => {
    expect(flagEntry('timesig:2')).toEqual({ kind: 'timesig', index: 2 })
    expect(flagEntry('mk_abc')).toBeNull()
  })

  it('puts a signature change on the bar line before the click', () => {
    const segments = meterSegments([{ tick: 0, timeSigNum: 4, timeSigDen: 4 }])
    // Bar three starts at 7680; a click half way into it lands there.
    expect(barStartAt(segments, 7680 + 1920)).toBe(7680)
    expect(barStartAt(segments, 100)).toBe(0)
  })

  it('counts bars in the signature that is in force', () => {
    // 4/4 for a bar, then 7/8 (3360 ticks a bar) from 3840.
    const segments = meterSegments([
      { tick: 0, timeSigNum: 4, timeSigDen: 4 },
      { tick: 3840, timeSigNum: 7, timeSigDen: 8 },
    ])
    expect(barStartAt(segments, 3840 + 3360 + 10)).toBe(3840 + 3360)
  })

  it('knows the tempo at a tick', () => {
    const entries = [entry(0, 140), entry(3840, 160)]
    expect(bpmAt(entries, 100)).toBe(140)
    expect(bpmAt(entries, 3840)).toBe(160)
  })
})
