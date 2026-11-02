import { describe, expect, it } from 'vitest'
import { audioChoiceChanged } from './SetupWizard'

const playing = { device: 'Speakers (Realtek)', sampleRate: 48000, bufferSize: 512 }

describe('Continue on the audio step of the setup wizard', () => {
  it('applies a device picked but not applied yet', () => {
    expect(audioChoiceChanged(playing, { ...playing, device: 'Focusrite USB' })).toBe(true)
    expect(audioChoiceChanged(playing, { ...playing, bufferSize: 256 })).toBe(true)
  })

  it('does nothing when the choice is what plays', () => {
    expect(audioChoiceChanged(playing, { ...playing })).toBe(false)
  })

  it('does nothing before the settings have been read', () => {
    expect(audioChoiceChanged(null, playing)).toBe(false)
  })
})
