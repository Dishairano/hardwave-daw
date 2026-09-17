import { describe, it, expect } from 'vitest'
import {
  PPQ,
  barBeatAtTick,
  gridLines,
  meterSegments,
  segmentAt,
  ticksPerBar,
  ticksPerBeat,
} from '../meter'

const fourFour = [{ tick: 0, timeSigNum: 4, timeSigDen: 4 }]

describe('ticksPerBeat', () => {
  it('counts a /8 signature in eighths', () => {
    expect(ticksPerBeat(4)).toBe(PPQ)
    expect(ticksPerBeat(8)).toBe(PPQ / 2)
    expect(ticksPerBar(7, 8)).toBe((PPQ * 7) / 2)
  })

  it('falls back to a quarter note rather than dividing by zero', () => {
    expect(ticksPerBeat(0)).toBe(PPQ)
  })
})

describe('meterSegments', () => {
  it('gives 4/4 from bar 1 for an empty map', () => {
    const [only] = meterSegments([])
    expect(only.startTick).toBe(0)
    expect(only.startBar).toBe(1)
    expect(only.ticksPerBar).toBe(PPQ * 4)
  })

  it('starts a segment only where the signature changes', () => {
    const segments = meterSegments([
      { tick: 0, timeSigNum: 4, timeSigDen: 4 },
      // A tempo point inherits the signature, so it is not a new bar.
      { tick: PPQ * 9, timeSigNum: 4, timeSigDen: 4 },
      { tick: PPQ * 16, timeSigNum: 3, timeSigDen: 4 },
    ])
    expect(segments.map(s => s.startTick)).toEqual([0, PPQ * 16])
    expect(segments[1].startBar).toBe(5)
  })

  it('counts a cut-short bar as a bar, as the engine does', () => {
    const segments = meterSegments([
      { tick: 0, timeSigNum: 4, timeSigDen: 4 },
      { tick: PPQ * 6, timeSigNum: 3, timeSigDen: 4 },
    ])
    // Bar 2 is cut in half by the change, so the new signature starts bar 3.
    expect(segments[1].startBar).toBe(3)
  })
})

describe('gridLines', () => {
  it('marks every fourth line as a bar in 4/4', () => {
    const lines = gridLines(meterSegments(fourFour), 0, PPQ * 8)
    expect(lines.filter(l => l.isBar).map(l => l.bar)).toEqual([1, 2, 3])
    expect(lines).toHaveLength(9)
  })

  it('changes the bar width where the signature changes', () => {
    const segments = meterSegments([
      { tick: 0, timeSigNum: 4, timeSigDen: 4 },
      { tick: PPQ * 8, timeSigNum: 3, timeSigDen: 4 },
    ])
    const bars = gridLines(segments, 0, PPQ * 14)
      .filter(l => l.isBar)
      .map(l => l.tick / PPQ)
    // Two bars of four beats, then bars of three.
    expect(bars).toEqual([0, 4, 8, 11, 14])
  })

  it('draws eighth-note beats in a /8 signature', () => {
    const lines = gridLines(meterSegments([{ tick: 0, timeSigNum: 7, timeSigDen: 8 }]), 0, PPQ * 7)
    expect(lines[1].tick).toBe(PPQ / 2)
    expect(lines.filter(l => l.isBar).map(l => l.tick)).toEqual([0, (PPQ * 7) / 2, PPQ * 7])
  })

  it('only returns lines inside the asked-for range', () => {
    const lines = gridLines(meterSegments(fourFour), PPQ * 4, PPQ * 6)
    expect(lines.map(l => l.tick / PPQ)).toEqual([4, 5, 6])
  })

  it('stops at the line budget instead of walking a whole song', () => {
    const lines = gridLines(meterSegments(fourFour), 0, PPQ * 100000, 50)
    expect(lines).toHaveLength(50)
  })
})

describe('barBeatAtTick', () => {
  it('keeps counting bars across a change', () => {
    const segments = meterSegments([
      { tick: 0, timeSigNum: 4, timeSigDen: 4 },
      { tick: PPQ * 16, timeSigNum: 3, timeSigDen: 4 },
    ])
    expect(barBeatAtTick(segments, 0)).toEqual({ bar: 1, beat: 1 })
    expect(barBeatAtTick(segments, PPQ * 16)).toEqual({ bar: 5, beat: 1 })
    expect(barBeatAtTick(segments, PPQ * 19).bar).toBe(6)
    expect(segmentAt(segments, PPQ * 20).timeSigNum).toBe(3)
  })
})
