/**
 * Waveform colours the way FL Studio's "Multiband" waveforms colour them,
 * for the views that hold a sample's audio themselves (the beat slicer and
 * the sample editor). The playlist, the sample window and the browser get
 * the same colours from the backend (`hardwave_dsp::waveform_colour`, which
 * has the measurements); the numbers here must stay the same as there.
 *
 * Red, green and blue are the levels of a low, a mid and a high band
 * (first-order crossovers at about 153 Hz and 1.31 kHz) divided by their
 * sum, over both channels, holding still over about 30 ms.
 */

/** Frames per stored set of band levels. */
export const HOP = 128

const LOW_MID_HZ = 153.34
const MID_HIGH_HZ = 1307.9
const MID_GAIN = 0.945
const HIGH_GAIN = 0.967
const SMOOTH_SECONDS = 0.03

/** What silence (or a bucket with no colour) is drawn in. */
export const SILENT_COLOUR = 'rgb(138,138,146)'

/** A colour from red, green and blue (0..255); all zero means silence. */
export function rgbColour(r: number, g: number, b: number): string {
  return r + g + b > 0 ? `rgb(${r | 0},${g | 0},${b | 0})` : SILENT_COLOUR
}

export interface WaveColours {
  /** The colour of frames `start..end`, or null for silence. */
  colour(start: number, end: number): [number, number, number] | null
}

/** Analyse a sample once; colour any stretch of it after. */
export function analyseColours(channels: Float32Array[], sampleRate: number): WaveColours {
  const frames = channels.reduce((n, c) => Math.max(n, c.length), 0)
  if (frames === 0 || sampleRate <= 0) return { colour: () => null }
  const hops = Math.ceil(frames / HOP)
  const sums = new Float64Array(hops * 3)
  const lowMid = Math.min(LOW_MID_HZ, sampleRate * 0.45)
  const midHigh = Math.min(MID_HIGH_HZ, sampleRate * 0.45)
  for (const channel of channels) {
    const lp1 = onePole(lowMid, sampleRate)
    const lp2 = onePole(midHigh, sampleRate)
    let x1a = 0, y1a = 0, x1b = 0, y1b = 0
    for (let i = 0; i < channel.length; i++) {
      const x = channel[i]
      const low = lp1.b * (x + x1a) - lp1.a * y1a
      x1a = x; y1a = low
      const rest = x - low
      const midRaw = lp2.b * (rest + x1b) - lp2.a * y1b
      x1b = rest; y1b = midRaw
      const mid = MID_GAIN * midRaw
      const high = HIGH_GAIN * (rest - midRaw)
      const at = ((i / HOP) | 0) * 3
      sums[at] += low * low
      sums[at + 1] += mid * mid
      sums[at + 2] += high * high
    }
  }
  const count = Math.max(1, channels.length)
  for (let h = 0; h < hops; h++) {
    const n = Math.min(HOP, frames - h * HOP) * count
    for (let k = 0; k < 3; k++) sums[h * 3 + k] /= n
  }
  const power = smooth(sums, hops, sampleRate)
  return {
    colour(start, end) {
      const first = Math.min(Math.floor(start / HOP), hops - 1)
      const last = Math.min(hops, Math.max(first + 1, Math.ceil(end / HOP)))
      let l = 0, m = 0, h = 0
      for (let i = first; i < last; i++) {
        l += power[i * 3]; m += power[i * 3 + 1]; h += power[i * 3 + 2]
      }
      const n = last - first
      l = Math.sqrt(l / n); m = Math.sqrt(m / n); h = Math.sqrt(h / n)
      const total = l + m + h
      if (total < 1e-7) return null
      const c = (v: number) => Math.min(255, Math.round((256 * v) / total))
      return [c(l), c(m), c(h)]
    },
  }
}

function onePole(hz: number, sampleRate: number): { b: number; a: number } {
  const k = Math.tan((Math.PI * hz) / sampleRate)
  return { b: k / (1 + k), a: (k - 1) / (1 + k) }
}

/** Smooth the hop powers with a Hann window SMOOTH_SECONDS long. */
function smooth(raw: Float64Array, hops: number, sampleRate: number): Float64Array {
  const len = Math.floor((SMOOTH_SECONDS * sampleRate) / HOP) | 1
  if (len < 3) return raw
  const window = Array.from({ length: len }, (_, i) => 0.5 - 0.5 * Math.cos((2 * Math.PI * (i + 0.5)) / len))
  const half = len >> 1
  const out = new Float64Array(hops * 3)
  for (let i = 0; i < hops; i++) {
    let l = 0, m = 0, h = 0, weight = 0
    for (let j = 0; j < len; j++) {
      const at = i + j - half
      if (at < 0) continue
      if (at >= hops) break
      const w = window[j]
      l += w * raw[at * 3]; m += w * raw[at * 3 + 1]; h += w * raw[at * 3 + 2]
      weight += w
    }
    out[i * 3] = l / weight; out[i * 3 + 1] = m / weight; out[i * 3 + 2] = h / weight
  }
  return out
}
