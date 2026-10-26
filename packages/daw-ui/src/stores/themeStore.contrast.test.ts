import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { highContrast, setHighContrast } from './themeStore'

describe('highContrast', () => {
  beforeEach(() => {
    const store = new Map<string, string>()
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => { store.set(k, v) },
      removeItem: (k: string) => { store.delete(k) },
    })
  })
  afterEach(() => vi.unstubAllGlobals())

  it('follows the setting once it is made', () => {
    vi.stubGlobal('window', { matchMedia: () => ({ matches: true }) })
    setHighContrast(false)
    expect(highContrast()).toBe(false)
    setHighContrast(true)
    expect(highContrast()).toBe(true)
  })

  it('follows Windows until it is set', () => {
    vi.stubGlobal('window', { matchMedia: (q: string) => ({ matches: q === '(prefers-contrast: more)' }) })
    expect(highContrast()).toBe(true)
    vi.stubGlobal('window', { matchMedia: () => ({ matches: false }) })
    expect(highContrast()).toBe(false)
  })
})
