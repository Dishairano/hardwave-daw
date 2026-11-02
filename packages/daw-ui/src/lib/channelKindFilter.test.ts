import { describe, expect, it } from 'vitest'
import { channelKindShown } from './channelKindFilter'

describe('the All / Audio / MIDI buttons over the channel rack', () => {
  it('All shows every channel', () => {
    for (const kind of ['Audio', 'Midi', 'Automation']) expect(channelKindShown('All', kind)).toBe(true)
  })

  it('Audio shows the sample channels only', () => {
    expect(channelKindShown('Audio', 'Audio')).toBe(true)
    expect(channelKindShown('Audio', 'Midi')).toBe(false)
  })

  it('MIDI shows the instrument channels only', () => {
    expect(channelKindShown('MIDI', 'Midi')).toBe(true)
    expect(channelKindShown('MIDI', 'Audio')).toBe(false)
    expect(channelKindShown('MIDI', 'Automation')).toBe(false)
  })
})
