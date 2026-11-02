import { describe, it, expect } from 'vitest'
import { crashChoiceForKey } from '../CrashRecoveryDialog'

describe('the crash recovery dialog keys', () => {
  it('Escape closes it without deleting the auto-saves', () => {
    expect(crashChoiceForKey('Escape')).toBe('later')
  })

  it('Enter recovers, and other keys do nothing', () => {
    expect(crashChoiceForKey('Enter')).toBe('recover')
    expect(crashChoiceForKey('a')).toBeNull()
  })
})
