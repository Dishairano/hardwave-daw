import { describe, it, expect } from 'vitest'
import {
  averageCurve, changesSound, describePitch, inKey, keyFromLabel, keyLabel, KEY_LABELS, movePitch,
  nearestInKey, pitchRange, playedLine, retime, snapPitch, splitNote, tuneAll,
  type ClipNotes, type TuneNote,
} from '../notesModel'

const note = (start: number, end: number, sung: number): TuneNote =>
  ({ start, end, sung, pitch: sung, drift: 1, newStart: start, newEnd: end, formants: true })

/** A take whose sung line sits at `pitch` from 0 to 2 s, a frame every 10 ms. */
const view = (notes: TuneNote[], line: (s: number) => number | null): ClipNotes => ({
  seconds: 2,
  notes,
  curveStart: 0,
  curveStep: 0.01,
  curve: Array.from({ length: 201 }, (_, i) => line(i * 0.01)),
  keyRoot: 5,
  keyMinor: true,
  tuneKey: '',
})

const fMinor = { root: 5, minor: true }

describe('the NOTES view', () => {
  it('names a pitch with its cents, as the side bar does', () => {
    expect(describePitch(63.38)).toBe('D#4 +38 cents')
    expect(describePitch(65)).toBe('F4 0 cents')
    expect(describePitch(66.8)).toBe('G4 -20 cents')
  })

  it('knows the notes of a key', () => {
    // F minor: F G Ab Bb C Db Eb.
    expect([65, 67, 68, 70, 72, 73, 75].every((p) => inKey(p, fMinor))).toBe(true)
    expect([66, 69, 71, 64].some((p) => inKey(p, fMinor))).toBe(false)
    expect(inKey(66, null)).toBe(true)
    expect(keyFromLabel('F minor')).toEqual(fMinor)
    expect(keyFromLabel('Chromatic')).toBeNull()
    expect(keyLabel(fMinor)).toBe('F minor')
    expect(KEY_LABELS).toHaveLength(25)
  })

  it('snaps a dragged note to the key, to a semitone, or not at all with Alt', () => {
    // The mockup's "go": sung D#4 +38, dragged up to about F4 +30.
    expect(snapPitch(65.3, 'scale', fMinor, false)).toBe(65)
    // E is not in F minor: the nearest in the key is Eb or F.
    expect([63, 65]).toContain(snapPitch(64.2, 'scale', fMinor, false))
    expect(snapPitch(64.2, 'semitone', fMinor, false)).toBe(64)
    expect(snapPitch(64.234, 'scale', fMinor, true)).toBe(64.23)
    expect(snapPitch(64.234, 'off', fMinor, false)).toBe(64.23)
    expect(nearestInKey(63.38, fMinor)).toBe(63)
  })

  it('tunes every note towards the key by the strength', () => {
    const notes = [note(0, 0.5, 65.08), note(0.6, 1, 63.38), note(1.1, 1.5, 66.4)]
    const full = tuneAll(notes, fMinor, 1)
    expect(full.map((n) => n.pitch)).toEqual([65, 63, 67])
    const half = tuneAll(notes, fMinor, 0.5)
    expect(half[1].pitch).toBeCloseTo(63.19, 5)
    // Where they were sung is kept, so they can be moved after.
    expect(half[1].sung).toBe(63.38)
  })

  it('moves the chosen notes together and no further than two octaves', () => {
    const notes = [note(0, 0.5, 60), note(0.6, 1, 62), note(1.1, 1.5, 64)]
    const moved = movePitch(notes, new Set([0, 2]), 2)
    expect(moved.map((n) => n.pitch)).toEqual([62, 62, 66])
    expect(movePitch(notes, new Set([1]), 40)[1].pitch).toBe(86)
    expect(changesSound(moved[0])).toBe(true)
    expect(changesSound(moved[1])).toBe(false)
  })

  it('retimes a note within its neighbours', () => {
    const notes = [note(0, 0.5, 60), note(0.6, 1, 62), note(1.1, 1.5, 64)]
    // The middle note's end pulled to 1.2 s stops at the next note.
    expect(retime(notes, 1, 'end', 1.2, 2)[1].newEnd).toBe(1.1)
    // Its start pulled back to 0.55 s is fine; to 0.3 s it stops at the first note.
    expect(retime(notes, 1, 'start', 0.55, 2)[1].newStart).toBe(0.55)
    expect(retime(notes, 1, 'start', 0.3, 2)[1].newStart).toBe(0.5)
    // Never shorter than a quarter of what was sung.
    expect(retime(notes, 1, 'end', 0.61, 2)[1].newEnd).toBeCloseTo(0.7, 9)
    // What was sung does not move.
    expect(retime(notes, 1, 'end', 1.05, 2)[1].end).toBe(1)
  })

  it('splits a note where two syllables were found as one', () => {
    const one = { ...note(0.2, 1.2, 61), pitch: 63 }
    const v = view([one], (s) => (s < 0.7 ? 60 : 62))
    const split = splitNote([one], 0, 0.7, v)
    expect(split).toHaveLength(2)
    expect(split[0].end).toBeCloseTo(0.7, 9)
    expect(split[1].start).toBeCloseTo(0.7, 9)
    // Each half is its own average, and keeps the move of two semitones.
    expect(split[0].sung).toBeCloseTo(60, 1)
    expect(split[1].sung).toBeCloseTo(62, 1)
    expect(split[0].pitch).toBeCloseTo(62, 1)
    expect(split[1].pitch).toBeCloseTo(64, 1)
    // Too close to an edge, nothing happens.
    expect(splitNote([one], 0, 0.21, v)).toEqual([one])
  })

  it('draws the line as it will play: moved, flattened, retimed', () => {
    const wobble = (s: number) => 60 + 0.3 * Math.sin(s * 30)
    const n = { ...note(0.5, 1, 60), pitch: 62, drift: 0.5, newEnd: 1.5 }
    const line = playedLine(n, view([n], wobble))
    expect(line.length).toBeGreaterThan(40)
    for (const [t, p] of line) {
      expect(t).toBeGreaterThanOrEqual(0.5 - 1e-9)
      expect(t).toBeLessThanOrEqual(1.5 + 1e-9)
      expect(Math.abs(p - 62)).toBeLessThanOrEqual(0.15 + 1e-9)
    }
    expect(averageCurve(view([n], () => null), 0, 1)).toBeNull()
  })

  it('shows at least an octave of rows around the notes', () => {
    expect(pitchRange([note(0, 1, 63.38), note(1, 2, 65.08)])).toEqual({ low: 59, high: 71 })
    const wide = pitchRange([note(0, 1, 50), { ...note(1, 2, 70), pitch: 75 }])
    expect(wide).toEqual({ low: 48, high: 77 })
  })
})
