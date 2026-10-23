import { describe, it, expect } from 'vitest'

const { usePatternStore, isHydrating } = await import('../patternStore')

describe('a rack loaded from a song is not an edit', () => {
  it('subscribers can tell a hydrate from an edit', () => {
    const seen: boolean[] = []
    const unsub = usePatternStore.subscribe(() => seen.push(isHydrating()))
    usePatternStore.getState().hydrate(null)
    usePatternStore.getState().setStep('ch-1', 0, 1)
    unsub()
    expect(seen[0]).toBe(true)
    expect(seen[seen.length - 1]).toBe(false)
    expect(isHydrating()).toBe(false)
  })
})
