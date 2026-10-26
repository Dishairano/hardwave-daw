import { describe, expect, it } from 'vitest'
import { tf, useLanguageStore } from './index'

describe('tf', () => {
  it('fills the blanks in the English and in a translation', () => {
    useLanguageStore.setState({ language: 'en' })
    expect(tf('{n} tracks on {name}', { n: 3, name: 'Bass' })).toBe('3 tracks on Bass')
    // A blank with no value stays visible rather than vanishing.
    expect(tf('{n} tracks', {})).toBe('{n} tracks')
  })
})
