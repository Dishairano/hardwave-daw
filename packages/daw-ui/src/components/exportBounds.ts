import type { ExportRange } from './ExportDialog'

/** What an export asks the engine to render: a stretch of the song in
 * frames at the export's rate (null for the whole song), or the channel
 * rack's pattern. */
export interface RenderBounds {
  startSamples: number | null
  endSamples: number | null
  pattern: boolean
}

export function renderBounds(
  range: ExportRange,
  loop: { start: number; end: number },
  projectSampleRate: number,
  sampleRate: number,
): RenderBounds {
  // The engine plays the pattern from its start and measures it under the
  // tempo map. Worked out here from one tempo and sent as a stretch of the
  // song, it rendered the song's first bars.
  if (range === 'pattern') return { startSamples: null, endSamples: null, pattern: true }
  if (range === 'loop' && loop.end > loop.start) {
    if (projectSampleRate <= 0 || projectSampleRate === sampleRate) {
      return { startSamples: loop.start, endSamples: loop.end, pattern: false }
    }
    const ratio = sampleRate / projectSampleRate
    return {
      startSamples: Math.round(loop.start * ratio),
      endSamples: Math.round(loop.end * ratio),
      pattern: false,
    }
  }
  return { startSamples: null, endSamples: null, pattern: false }
}
