import { beforeEach, describe, expect, it, vi } from 'vitest'

// The engine's side: which track the keyboard plays, and the order things
// arrive in.
const log: string[] = []
let keyboard: string | null = null
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args?: { trackId?: string | null; event?: { kind: string } }) => {
    switch (cmd) {
      case 'get_keyboard_track':
        return keyboard
      case 'set_keyboard_track':
        keyboard = args?.trackId ?? null
        log.push(`route ${keyboard}`)
        return null
      case 'inject_midi_event':
        log.push(`${args?.event?.kind} to ${keyboard}`)
        return null
      default:
        return null
    }
  }),
}))

const { previewChannel, stepPreviewMs, SETTLE_MS } = await import('./channelPreview')

describe('hearing a channel from the channel rack', () => {
  beforeEach(() => {
    log.length = 0
    keyboard = 'kick'
    vi.useFakeTimers()
  })

  it('plays the keyboard channel straight away and leaves the routing alone', async () => {
    const done = previewChannel('kick', 0.85, 100)
    await vi.runAllTimersAsync()
    await done
    expect(log).toEqual(['note_on to kick', 'note_off to kick'])
    expect(keyboard).toBe('kick')
  })

  it('points the keyboard at another channel for the note, then back', async () => {
    const done = previewChannel('clap', 0.85, 100)
    await vi.runAllTimersAsync()
    await done
    expect(log).toEqual(['route clap', 'note_on to clap', 'note_off to clap', 'route kick'])
    expect(keyboard).toBe('kick')
  })

  it('waits for the routing to land before the note, and for the note-off before routing back', async () => {
    const done = previewChannel('hat', 0.85, 100)
    await vi.advanceTimersByTimeAsync(SETTLE_MS - 1)
    expect(log).toEqual(['route hat'])
    await vi.advanceTimersByTimeAsync(1)
    expect(log).toEqual(['route hat', 'note_on to hat'])
    await vi.advanceTimersByTimeAsync(100)
    expect(log).toEqual(['route hat', 'note_on to hat', 'note_off to hat'])
    await vi.advanceTimersByTimeAsync(SETTLE_MS)
    await done
    expect(log[log.length - 1]).toBe('route kick')
  })

  it('never sends one channel\'s note-off to the next one', async () => {
    previewChannel('clap', 0.85, 100)
    const done = previewChannel('snare', 0.85, 100)
    await vi.runAllTimersAsync()
    await done
    expect(log).toEqual([
      'route clap', 'note_on to clap', 'note_off to clap', 'route kick',
      'route snare', 'note_on to snare', 'note_off to snare', 'route kick',
    ])
  })

  it('keeps a channel picked during the note as the keyboard\'s', async () => {
    const done = previewChannel('clap', 0.85, 100)
    await vi.advanceTimersByTimeAsync(SETTLE_MS + 10)
    keyboard = 'snare' // the producer clicked Snare's name meanwhile
    await vi.runAllTimersAsync()
    await done
    expect(keyboard).toBe('snare')
  })

  it('lasts one step at the tempo, kept short', () => {
    expect(stepPreviewMs(140)).toBeCloseTo(107.1, 0)
    expect(stepPreviewMs(60)).toBe(120)
    expect(stepPreviewMs(400)).toBeCloseTo(37.5, 1)
  })
})
