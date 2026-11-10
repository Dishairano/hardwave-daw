import { describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn() }))

const { channelTip } = await import('./ChannelNameTip')
const { channelSound } = await import('../../stores/instrumentChannels')
type Track = Parameters<typeof channelTip>[0]

const sampler = (samplePath?: string) => ({ id: 's1', pluginId: 'hardwave.native.sampler', pluginName: 'Hardwave Sampler', enabled: true, wet: 1, sidechainSource: null, samplePath })
const track = (over: Partial<Track>): Track => ({ id: 't', name: 'Clap', kind: 'Midi', inserts: [], ...over }) as Track

describe('what a channel says it plays', () => {
  it('names the sample in a sampler channel', () => {
    const clap = track({ inserts: [sampler('C:\\Program Files\\Hardwave DAW\\drums\\Clap 1.wav')] })
    expect(channelSound(clap)).toBe('Clap 1')
    expect(channelSound(clap, true)).toBe('Clap 1.wav')
    expect(channelTip(clap)).toEqual({
      name: 'Clap',
      what: 'Clap 1.wav in Hardwave Sampler.',
      how: 'Click a step or the name to hear it. Drop another sample on the channel to swap it.',
    })
  })

  it('names the kick synth, and says how to open its editor', () => {
    const kick = track({ name: 'Kick', instrument: 'kick_synth' })
    expect(channelSound(kick)).toBe('Kick synth')
    expect(channelTip(kick)?.how).toContain('Double-click the name for its editor.')
  })

  it('does not claim a sample a sampler has not got', () => {
    const empty = track({ inserts: [sampler()] })
    expect(channelSound(empty)).toBeNull()
    expect(channelTip(empty)?.how).toBe('Drop a sample on the channel to load it.')
  })

  it('leaves the label off where it would only repeat the name', () => {
    const synth = track({ name: 'Hardwave 3xOSC', inserts: [{ ...sampler(), pluginId: 'hardwave.native.tripleosc', pluginName: 'Hardwave 3xOSC' }] })
    expect(channelSound(synth)).toBeNull()
    expect(channelTip(synth, 'Hardwave 3xOSC')).toEqual({ name: 'Hardwave 3xOSC', what: 'Hardwave 3xOSC.', how: 'Click a step or the name to hear it.' })
    expect(channelTip(track({ kind: 'Audio' }))).toBeNull()
  })
})
