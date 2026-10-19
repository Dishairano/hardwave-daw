import { describe, expect, it } from 'vitest'
import { analyseColours, rgbColour, SILENT_COLOUR } from './waveformColour'

// The same FL Studio colours the backend's tests hold the Rust analysis to.
const RATE = 48_000

function tone(hz: number, amp: number, seconds = 0.5): Float32Array {
  const n = Math.round(seconds * RATE)
  return Float32Array.from({ length: n }, (_, i) => amp * Math.sin((2 * Math.PI * hz * i) / RATE))
}

function middle(channels: Float32Array[]): [number, number, number] {
  const n = channels[0].length
  const c = analyseColours(channels, RATE).colour(n / 5, (n * 4) / 5)
  if (!c) throw new Error('silent')
  return c
}

function distance(a: [number, number, number], hex: number): number {
  const b = [(hex >> 16) & 255, (hex >> 8) & 255, hex & 255]
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2])
}

describe('waveform colours', () => {
  it('gives pure tones the colour FL draws', () => {
    for (const [hz, fl] of [[25, 0xdc2201], [50, 0xbe3907], [120, 0x8d650d], [500, 0x34913c], [2000, 0x0f5b97], [5600, 0x072bcf]]) {
      expect(distance(middle([tone(hz, 0.5)]), fl), `${hz} Hz`).toBeLessThan(10)
    }
  })

  it('makes a low and a high tone together the dim purple FL draws', () => {
    const high = tone(5000, 0.25)
    const mix = tone(50, 0.25).map((v, i) => v + high[i])
    expect(distance(middle([mix]), 0x692a6b)).toBeLessThan(10)
  })

  it('counts highs that are only in the side signal', () => {
    const bass = tone(45, 0.4)
    const hiss = tone(6000, 0.2)
    const left = bass.map((v, i) => v + hiss[i])
    const right = bass.map((v, i) => v - hiss[i])
    expect(middle([left, right])[2]).toBeGreaterThan(middle([bass, bass])[2] + 40)
  })

  it('has no colour for silence', () => {
    expect(analyseColours([new Float32Array(4096)], RATE).colour(0, 4096)).toBeNull()
    expect(analyseColours([], RATE).colour(0, 10)).toBeNull()
    expect(rgbColour(0, 0, 0)).toBe(SILENT_COLOUR)
    expect(rgbColour(190, 57, 7)).toBe('rgb(190,57,7)')
  })
})
