/**
 * The sample window's NOTES view, without the drawing: a take's sung notes
 * as `find_clip_notes` hands them over, and what moving one does to them.
 * Approved from the mockup on the suite (daw-tune-notes-mockup): notes as
 * blobs on a piano grid, dragged up or down and snapped to the key, their
 * edges dragged to retime them, Alt+click to split one, double-click to put
 * it on the key, a Drift amount, and Tune all.
 *
 * Times are seconds of the file; pitches are MIDI notes with the cents in
 * the fraction (63.38 is D#4, 38 cents sharp).
 */

/** One note, as sung and as it plays now (`NoteInfo` in tune_notes.rs). */
export interface TuneNote {
  start: number
  end: number
  sung: number
  pitch: number
  /** How much of the wobble is kept: 1 as sung, 0 flat. */
  drift: number
  newStart: number
  newEnd: number
  /** The voice keeps its character as the pitch moves. */
  formants: boolean
}

/** What `find_clip_notes` hands back. */
export interface ClipNotes {
  seconds: number
  notes: TuneNote[]
  /** Frame i of the sung pitch is at curveStart + i * curveStep seconds. */
  curveStart: number
  curveStep: number
  curve: (number | null)[]
  keyRoot: number | null
  keyMinor: boolean
  tuneKey: string
}

/** A key, or null for every semitone. */
export type Key = { root: number; minor: boolean } | null
export type Snap = 'scale' | 'semitone' | 'off'

const NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
const MAJOR = [0, 2, 4, 5, 7, 9, 11]
const MINOR = [0, 2, 3, 5, 7, 8, 10]
/** How far a note may move, and be stretched, as tune_notes.rs allows. */
export const MAX_SEMITONES = 24
export const MAX_STRETCH = 4
/** The shortest a note can be made or split into, in seconds. */
export const MIN_NOTE = 0.03
/** How far from its note the sung line is taken as wobble (note_tune.rs). */
const DRIFT_REACH = 2
/** A note this many cents off its nearest semitone shows as out of tune. */
export const OUT_OF_TUNE_CENTS = 20

/** C4 is 60, as the piano roll names its keys. */
export const noteName = (pitch: number) => {
  const n = Math.round(pitch)
  return `${NAMES[((n % 12) + 12) % 12]}${Math.floor(n / 12) - 1}`
}
/** Cents from the nearest semitone, -50..50. */
export const centsOff = (pitch: number) => Math.round((pitch - Math.round(pitch)) * 100)
/** "D#4 +38 cents". */
export const describePitch = (pitch: number) => {
  const c = centsOff(pitch)
  return `${noteName(pitch)} ${c > 0 ? '+' : ''}${c} cents`
}
export const keyLabel = (key: Key) => (key ? `${NAMES[key.root]} ${key.minor ? 'minor' : 'major'}` : 'Chromatic')
/** Every key, as the Key box lists them. */
export const KEY_LABELS: string[] = ['Chromatic', ...NAMES.flatMap((n) => [`${n} major`, `${n} minor`])]
export const keyFromLabel = (label: string): Key => {
  const m = /^([A-G]#?) (major|minor)$/.exec(label)
  return m ? { root: NAMES.indexOf(m[1]), minor: m[2] === 'minor' } : null
}

export function inKey(pitch: number, key: Key): boolean {
  if (!key) return true
  const degree = (((Math.round(pitch) - key.root) % 12) + 12) % 12
  return (key.minor ? MINOR : MAJOR).includes(degree)
}

/** The note of the key nearest a pitch; any semitone without a key. A tie
 *  goes to the note below, as a sung note sits flat more often than sharp. */
export function nearestInKey(pitch: number, key: Key): number {
  const below = Math.floor(pitch)
  let best = Math.round(pitch)
  let distance = Infinity
  for (let p = below - 6; p <= below + 7; p++) {
    if (!inKey(p, key)) continue
    const d = Math.abs(p - pitch)
    if (d < distance - 1e-9) { best = p; distance = d }
  }
  return best
}

/** Where a dragged note lands: on the key, on a semitone, or where it was
 *  let go (Alt, or snap off). */
export function snapPitch(raw: number, snap: Snap, key: Key, free: boolean): number {
  if (free || snap === 'off') return Math.round(raw * 100) / 100
  return snap === 'scale' ? nearestInKey(raw, key) : Math.round(raw)
}

/** Whether playing a note differs from the take (TunedNote::changes_sound). */
export function changesSound(n: TuneNote): boolean {
  return Math.abs(n.pitch - n.sung) > 0.005 || n.drift < 0.999
    || Math.abs(n.newStart - n.start) > 1e-5 || Math.abs(n.newEnd - n.end) > 1e-5
}

/** Tune all: every note towards the nearest note of the key, by `strength`
 *  (1 is all the way), as Tune this take does. */
export function tuneAll(notes: TuneNote[], key: Key, strength: number): TuneNote[] {
  return notes.map((n) => {
    const target = nearestInKey(n.sung, key)
    return { ...n, pitch: n.sung + (target - n.sung) * strength }
  })
}

/** Move the chosen notes by the same interval, each kept within reach of
 *  where it was sung. */
export function movePitch(notes: TuneNote[], chosen: Set<number>, by: number): TuneNote[] {
  return notes.map((n, i) => {
    if (!chosen.has(i)) return n
    const pitch = Math.max(n.sung - MAX_SEMITONES, Math.min(n.sung + MAX_SEMITONES, n.pitch + by))
    return { ...n, pitch: Math.max(0, Math.min(127, pitch)) }
  })
}

/** A note's start or end moved to `to` (seconds), kept clear of its
 *  neighbours and no shorter or longer than the limits allow. */
export function retime(notes: TuneNote[], index: number, edge: 'start' | 'end', to: number, seconds: number): TuneNote[] {
  const n = notes[index]
  if (!n) return notes
  const sung = n.end - n.start
  const shortest = Math.max(MIN_NOTE, sung / MAX_STRETCH)
  const longest = sung * MAX_STRETCH
  const next = { ...n }
  if (edge === 'start') {
    const floor = Math.max(index > 0 ? notes[index - 1].newEnd : 0, n.newEnd - longest)
    next.newStart = Math.max(floor, Math.min(n.newEnd - shortest, to))
  } else {
    const ceiling = Math.min(index + 1 < notes.length ? notes[index + 1].newStart : seconds, n.newStart + longest)
    next.newEnd = Math.min(ceiling, Math.max(n.newStart + shortest, to))
  }
  return notes.map((m, i) => (i === index ? next : m))
}

/** Where in the take a moment of the result comes from, inside a note. */
const sourceOf = (n: TuneNote, t: number) =>
  n.start + ((t - n.newStart) * (n.end - n.start)) / Math.max(1e-9, n.newEnd - n.newStart)
const resultOf = (n: TuneNote, s: number) =>
  n.newStart + ((s - n.start) * (n.newEnd - n.newStart)) / Math.max(1e-9, n.end - n.start)

/** Split a note in two at `at` (seconds of the result), where two
 *  syllables were found as one. Each half's sung pitch is its own average,
 *  and each keeps the move the note had. Unchanged when either half would
 *  be too short. */
export function splitNote(notes: TuneNote[], index: number, at: number, view: ClipNotes): TuneNote[] {
  const n = notes[index]
  if (!n || at - n.newStart < MIN_NOTE || n.newEnd - at < MIN_NOTE) return notes
  const s = sourceOf(n, at)
  const moved = n.pitch - n.sung
  const half = (start: number, end: number, newStart: number, newEnd: number): TuneNote => {
    const sung = averageCurve(view, start, end) ?? n.sung
    return { ...n, start, end, newStart, newEnd, sung, pitch: sung + moved }
  }
  return [
    ...notes.slice(0, index),
    half(n.start, s, n.newStart, at),
    half(s, n.end, at, n.newEnd),
    ...notes.slice(index + 1),
  ]
}

/** The sung pitch at a moment of the take, or null where nothing is sung. */
export function curveAt(view: ClipNotes, s: number): number | null {
  const f = (s - view.curveStart) / view.curveStep
  const i = Math.floor(f)
  const a = view.curve[i] ?? null
  const b = view.curve[i + 1] ?? null
  if (a === null || b === null) return f - i < 0.5 ? a : b
  return a + (b - a) * (f - i)
}

/** The average sung pitch between two moments, or null. */
export function averageCurve(view: ClipNotes, from: number, to: number): number | null {
  const first = Math.max(0, Math.ceil((from - view.curveStart) / view.curveStep))
  const last = Math.min(view.curve.length - 1, Math.floor((to - view.curveStart) / view.curveStep))
  let sum = 0
  let count = 0
  for (let i = first; i <= last; i++) {
    const p = view.curve[i]
    if (p !== null && p !== undefined) { sum += p; count++ }
  }
  return count ? sum / count : null
}

/** The line through a note as it will play: the sung line, moved with the
 *  note, its wobble scaled by the drift kept, at the note's new times.
 *  Pairs of (seconds of the result, pitch). */
export function playedLine(n: TuneNote, view: ClipNotes): [number, number][] {
  const out: [number, number][] = []
  const first = Math.max(0, Math.ceil((n.start - view.curveStart) / view.curveStep))
  const last = Math.min(view.curve.length - 1, Math.floor((n.end - view.curveStart) / view.curveStep))
  for (let i = first; i <= last; i++) {
    const c = view.curve[i]
    if (c === null || c === undefined) continue
    const s = view.curveStart + i * view.curveStep
    const wobble = c - n.sung
    const pitch = Math.abs(wobble) > DRIFT_REACH ? c + n.pitch - n.sung : n.pitch + n.drift * wobble
    out.push([resultOf(n, s), pitch])
  }
  return out
}

/** The pitch rows the grid shows: every note, as sung and as now, with
 *  room either side, an octave at least. */
export function pitchRange(notes: TuneNote[]): { low: number; high: number } {
  if (!notes.length) return { low: 57, high: 69 }
  let low = Infinity
  let high = -Infinity
  for (const n of notes) {
    low = Math.min(low, n.sung, n.pitch)
    high = Math.max(high, n.sung, n.pitch)
  }
  low = Math.floor(low) - 2
  high = Math.ceil(high) + 2
  if (high - low < 12) {
    const grow = 12 - (high - low)
    low -= Math.floor(grow / 2)
    high += Math.ceil(grow / 2)
  }
  return { low: Math.max(0, low), high: Math.min(127, high) }
}
