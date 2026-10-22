import { describe, expect, it } from 'vitest'
import { laneDefinition, laneDisplayValue, laneDefaultValue } from './midiCcStore'

describe('controller lanes', () => {
  it('has a channel aftertouch lane, read 0 to 127 and starting at rest', () => {
    const at = laneDefinition('at')
    expect(at?.kind).toBe('channelPressure')
    expect(laneDisplayValue(at!, 1)).toBe('127')
    expect(laneDefaultValue(at!)).toBe(0)
  })

  it('still reads a plain CC number', () => {
    expect(laneDefinition('cc74')).toMatchObject({ kind: 'cc', cc: 74 })
  })
})
