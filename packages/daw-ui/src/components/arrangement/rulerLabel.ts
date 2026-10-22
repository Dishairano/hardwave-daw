/** What the playlist ruler counts in. */
export type RulerUnits = 'bars' | 'time' | 'timecode' | 'samples'
/** Frame rates the timecode ruler and the MIDI timecode sender share. */
export type TimecodeFps = 24 | 25 | 30

const PPQ = 960

/**
 * The ruler label over a bar line. The time is worked out from the same
 * tick the bar line sits on, through the project tempo, so the readings
 * never disagree with the bars.
 */
export function rulerLabel(
  tick: number,
  bar: number,
  units: RulerUnits,
  bpm: number,
  fps: TimecodeFps,
  sampleRate: number,
): string {
  if (units === 'bars') return String(bar)
  const total = (tick / PPQ) * (60 / Math.max(1, bpm))
  if (units === 'samples') return String(Math.round(total * sampleRate))
  const minutes = Math.floor(total / 60)
  const seconds = total - minutes * 60
  if (units === 'time') return `${minutes}:${seconds.toFixed(1).padStart(4, '0')}`
  const pad = (n: number) => String(n).padStart(2, '0')
  // The frame the line falls in, counted from the whole time, so a frame
  // never reads as fps (the tiny allowance keeps 2.0 s from reading as
  // 1 s and the last frame through float error).
  const allFrames = Math.floor(total * fps + 1e-6)
  const frames = allFrames % fps
  const secs = Math.floor(allFrames / fps)
  return `${pad(Math.floor(secs / 60))}:${pad(secs % 60)}:${pad(frames)}`
}

/** Labels this wide need more room before they overlap. */
export function labelGap(units: RulerUnits, base: number): number {
  return units === 'bars' ? base : units === 'samples' ? base * 2.6 : base * 2
}
