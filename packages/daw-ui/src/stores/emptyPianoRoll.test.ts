import { beforeEach, describe, expect, it, vi } from 'vitest'

// A small engine: tracks, clips and notes, enough for the calls the empty
// piano roll makes.
type Call = [string, Record<string, unknown> | undefined]
const calls: Call[] = []
let nextId = 0
const engineTracks: { id: string; kind: string; name: string; clips: { id: string; kind: string }[]; inserts: { id: string; pluginId: string }[]; instrument?: string }[] = []

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push([cmd, args])
    switch (cmd) {
      case 'add_midi_track': {
        const id = `track-${++nextId}`
        engineTracks.push({ id, kind: 'Midi', name: String(args?.name), clips: [], inserts: [] })
        return id
      }
      case 'add_plugin_to_track': {
        const slot = `slot-${++nextId}`
        engineTracks.find(t => t.id === args?.trackId)?.inserts.push({ id: slot, pluginId: String(args?.pluginId) })
        return slot
      }
      case 'set_track_instrument': {
        const t = engineTracks.find(t => t.id === args?.trackId)
        if (t) t.instrument = String(args?.kind)
        return null
      }
      case 'create_midi_clip': {
        const clip = `clip-${++nextId}`
        engineTracks.find(t => t.id === args?.trackId)?.clips.push({ id: clip, kind: 'midi' })
        return clip
      }
      case 'add_midi_note':
        return 0
      case 'get_tracks_with_clips':
        return engineTracks.map(t => ({ ...t, clips: t.clips.map(c => ({ ...c })), inserts: t.inserts.map(s => ({ ...s, pluginName: s.pluginId })) }))
      default:
        return null
    }
  }),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn() }))

const { useTrackStore } = await import('./trackStore')
const { addInstrumentToPianoRoll, addSynthWithNote, followSelectionInPianoRoll, openChannelInPianoRoll } = await import('./pianoRollBinding')
const { BUILT_IN_INSTRUMENTS } = await import('../components/piano-roll/EmptyRollPicker')

const settle = () => new Promise(r => setTimeout(r, 0))
const named = (cmd: string) => calls.filter(c => c[0] === cmd)

describe('the empty piano roll', () => {
  beforeEach(async () => {
    calls.length = 0
    engineTracks.length = 0
    engineTracks.push({ id: 'insert-001', kind: 'Midi', name: 'Kick', clips: [], inserts: [], instrument: 'kick_synth' })
    useTrackStore.setState({ selectedTrackId: null, activeMidiTrackId: null, activeMidiClipId: null })
    await useTrackStore.getState().fetchTracks()
  })

  it('opens on a channel in the song, giving it a clip once', async () => {
    // The roll follows the selection while it is open; picking a channel
    // from the roll must not make a second clip through that path.
    const stop = followSelectionInPianoRoll()
    await openChannelInPianoRoll('insert-001')
    await settle()
    stop()
    expect(named('create_midi_clip')).toHaveLength(1)
    const s = useTrackStore.getState()
    expect(s.activeMidiTrackId).toBe('insert-001')
    expect(s.activeMidiClipId).toBe(engineTracks[0].clips[0].id)
    expect(s.selectedTrackId).toBe('insert-001')
  })

  it('adds an instrument as a new channel and opens it', async () => {
    const id = await addInstrumentToPianoRoll({ pluginId: 'hardwave.native.wavetable', name: 'Hardwave Wavetable' })
    expect(named('add_midi_track')[0][1]).toEqual({ name: 'Hardwave Wavetable' })
    expect(named('add_plugin_to_track')[0][1]).toEqual({ trackId: id, pluginId: 'hardwave.native.wavetable' })
    expect(useTrackStore.getState().activeMidiTrackId).toBe(id)
    expect(useTrackStore.getState().activeMidiClipId).not.toBeNull()
  })

  it('adds the kick synth as a channel of its own', async () => {
    const id = await addInstrumentToPianoRoll('kick_synth')
    expect(named('set_track_instrument')[0][1]).toEqual({ trackId: id, kind: 'kick_synth' })
    expect(named('add_plugin_to_track')).toHaveLength(0)
    expect(useTrackStore.getState().activeMidiTrackId).toBe(id)
  })

  it('turns a click in the empty grid into 3xOSC with the note, before the roll opens on it', async () => {
    const id = await addSynthWithNote({ pitch: 64, startTick: 480, durationTicks: 240, velocity: 0.8 })
    expect(named('add_plugin_to_track')[0][1]).toEqual({ trackId: id, pluginId: 'hardwave.native.tripleosc' })
    const note = named('add_midi_note')[0][1]
    expect(note).toMatchObject({ trackId: id, pitch: 64, startTick: 480, durationTicks: 240, velocity: 0.8 })
    // The note went in before the clip was opened, so the roll's first
    // read of the clip has it.
    const order = calls.map(c => c[0])
    expect(order.indexOf('add_midi_note')).toBeLessThan(order.lastIndexOf('set_keyboard_track'))
    expect(useTrackStore.getState().activeMidiClipId).toBe(note?.clipId)
  })
})

describe("the roll's list of Hardwave instruments", () => {
  // The plug-ins the factory makes, read from their descriptors: an effect
  // in this list (the sub bass is one) would add a channel that plays nothing.
  // Read through Vite's glob, not node:fs, so the app's typecheck (which has
  // no Node types) accepts this file.
  const sources = import.meta.glob<string>('../../../../crates/hardwave-native-plugins/src/*.rs', {
    query: '?raw',
    import: 'default',
    eager: true,
  })
  const instruments = new Set<string>()
  for (const text of Object.values(sources)) {
    const id = /const ID: &'static str = "([^"]+)"|const ID: &str = "([^"]+)"/.exec(text)
    if (id && /PluginCategory::Instrument/.test(text)) instruments.add(id[1] ?? id[2])
  }

  it('offers only plug-ins the factory makes as instruments, and the kick synth', () => {
    expect(instruments.size).toBeGreaterThanOrEqual(5)
    for (const b of BUILT_IN_INSTRUMENTS) {
      if (b.add === 'kick_synth') continue
      expect(instruments, b.name).toContain(b.add.pluginId)
    }
    expect(BUILT_IN_INSTRUMENTS.map(b => b.name)).not.toContain('Hardwave Sub Bass')
    expect(BUILT_IN_INSTRUMENTS[0].add).toEqual({ pluginId: 'hardwave.native.tripleosc', name: 'Hardwave 3xOSC' })
  })
})
