import { describe, it, expect } from 'vitest'
import { interfaceInputs, inputValue, parseInputValue } from './recordInputs'

describe('interfaceInputs', () => {
  it('lists the pairs, then every channel on its own', () => {
    expect(interfaceInputs(4).map(o => o.label)).toEqual([
      'In 1-2', 'In 3-4', 'In 1', 'In 2', 'In 3', 'In 4',
    ])
  })

  it('has no pair for an odd last channel', () => {
    expect(interfaceInputs(3).map(o => o.label)).toEqual(['In 1-2', 'In 1', 'In 2', 'In 3'])
    expect(interfaceInputs(0)).toEqual([])
  })
})

describe('input values', () => {
  it('round-trip, and anything else is not an interface input', () => {
    const ch = { first: 2, mono: true }
    expect(parseInputValue(inputValue(ch))).toEqual(ch)
    expect(parseInputValue('master')).toBeNull()
    expect(parseInputValue('')).toBeNull()
  })
})
