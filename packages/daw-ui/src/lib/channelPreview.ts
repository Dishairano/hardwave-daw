import { invoke } from '@tauri-apps/api/core'

/**
 * Hearing a channel from the channel rack: a click on its name, or on a step
 * that turns on, plays the channel's sound once, with the song stopped too.
 *
 * The note travels the way a MIDI keyboard's notes do, to the channel the
 * keyboard plays. For another channel the keyboard is pointed at it for the
 * length of the note and then pointed back. The engine takes a new routing at
 * the start of an audio block and the notes after it, so a note is sent only
 * once the routing has had time to land, and the routing goes back only once
 * the note's end has gone the same way. Previews run one after another, so
 * one channel's note-off never reaches the next channel.
 */

/** The note a step plays (the steps play middle C). */
export const STEP_NOTE = 60
/** Long enough for a few audio blocks at any buffer size the DAW offers. */
export const SETTLE_MS = 40

const sleep = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms))

let chain: Promise<void> = Promise.resolve()

async function play(trackId: string, velocity: number, lengthMs: number): Promise<void> {
  const before = await invoke<string | null>('get_keyboard_track')
  const moved = before !== trackId
  if (moved) {
    await invoke('set_keyboard_track', { trackId })
    await sleep(SETTLE_MS)
  }
  await invoke('inject_midi_event', { event: { kind: 'note_on', channel: 0, note: STEP_NOTE, velocity } })
  await sleep(lengthMs)
  await invoke('inject_midi_event', { event: { kind: 'note_off', channel: 0, note: STEP_NOTE } })
  if (!moved) return
  await sleep(SETTLE_MS)
  // Picking another channel meanwhile moved the keyboard on purpose; that
  // routing stays.
  if ((await invoke<string | null>('get_keyboard_track')) === trackId) {
    await invoke('set_keyboard_track', { trackId: before })
  }
}

/** Play one step's note on an instrument channel. Resolves when it is over. */
export function previewChannel(trackId: string, velocity: number, lengthMs: number): Promise<void> {
  chain = chain.then(() => play(trackId, velocity, lengthMs)).catch(() => {})
  return chain
}

/** One step's length at this tempo, kept short so quick clicks keep up. */
export function stepPreviewMs(bpm: number): number {
  const step = 60_000 / Math.max(20, bpm) / 4
  return Math.min(120, Math.max(30, step))
}
