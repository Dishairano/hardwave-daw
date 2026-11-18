import { describe, expect, it } from 'vitest'
import picomatch from 'picomatch'

// vite, vitest, tinyglobby and fdir all load `picomatch`. package.json
// overrides it with vendor/picomatch, picomatch 4.0.7 with the fix for
// GHSA-fc5q-39xc-vjpf: a repeated extglob with one branch, such as `+(*a)`,
// backtracked exponentially on a string that almost matches. This checks
// that the build tools get the fixed copy, and that it answers at once.
describe('picomatch, as the build tools load it', () => {
  it('is the fixed copy from vendor/picomatch', () => {
    // Upstream compiles this to `(?:[^/]*?a)+`; the fix drops the
    // repetition, which cannot match more than one `*a` does.
    expect(picomatch.makeRe('+(*a)').source).toBe('^(?:(?=.)(?:[^/]*?a))$')
  })

  it('answers the advisory patterns at once', () => {
    // Upstream takes seconds on each of these 25-character strings, and
    // twice as long for every character more.
    const cases: [string, string][] = [
      ['+(*)', 'a'.repeat(24) + '/'],
      ['+(*a)', 'a'.repeat(24) + 'b'],
      ['*(*a)', 'a'.repeat(24) + 'b'],
      ['+(@(*a))', 'a'.repeat(24) + 'b'],
      ['+(!(b))', 'a'.repeat(24) + '/'],
      ['+(a+)', 'a'.repeat(24) + 'b'],
    ]
    for (const [glob, almost] of cases) {
      const start = performance.now()
      picomatch.makeRe(glob).test(almost)
      expect(performance.now() - start, glob).toBeLessThan(100)
    }
  })

  it('still matches what the DAW config asks for', () => {
    expect(picomatch.isMatch('src/lib/x.test.ts', 'src/**/*.test.{ts,tsx}')).toBe(true)
    expect(picomatch.isMatch('src/dev/x.ts', 'src/**/*.test.{ts,tsx}')).toBe(false)
    expect(picomatch.isMatch('banana', '+(*a)')).toBe(true)
    expect(picomatch.isMatch('bananas', '+(*a)')).toBe(false)
  })
})
