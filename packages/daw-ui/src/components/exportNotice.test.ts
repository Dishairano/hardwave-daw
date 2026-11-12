import { describe, expect, it } from 'vitest'
import { describeExport, settleExport } from './exportNotice'

describe('the export notice', () => {
  it('names the format and the loudness of the file', () => {
    const n = describeExport({
      path: 'C:/songs/drop.flac',
      duration_secs: 192,
      loudness: { peakDb: -1.2, truePeakDb: -1.0, lufsIntegrated: -7.24 },
    })
    expect(n.title).toBe('Exported 192.0 s to FLAC: -7.2 LUFS, true peak -1.0 dBTP')
    expect(n.over).toBe(false)
  })

  it('warns about a true peak over 0 dB, and says MP3 numbers are before encoding', () => {
    const n = describeExport({
      path: 'drop.mp3',
      duration_secs: 10,
      loudness: { peakDb: -0.1, truePeakDb: 0.4, lufsIntegrated: -5.0 },
    })
    expect(n.title).toContain('true peak +0.4 dBTP (before encoding)')
    expect(n.over).toBe(true)
    expect(n.detail).toContain('-1 dB')
  })

  it('still says something for a cancelled or old-style result', () => {
    expect(describeExport({ path: 'x.wav', duration_secs: 1 }).title).toBe('Exported 1.0 s to WAV')
  })
})

describe('the end of an export', () => {
  it('closes quietly when the render was cancelled, with no error box', async () => {
    const seen: string[] = []
    await settleExport({ path: 'x.wav', duration_secs: 1, cancelled: true }, {
      finished: () => { seen.push('finished') },
      cancelled: () => { seen.push('cancelled') },
    })
    expect(seen).toEqual(['cancelled'])
  })

  it('hands a finished file on, after what it waits for', async () => {
    const seen: string[] = []
    await settleExport({ path: 'x.wav', duration_secs: 1, cancelled: false }, {
      finished: async (r) => { await Promise.resolve(); seen.push(`finished ${r.path}`) },
      cancelled: () => { seen.push('cancelled') },
    })
    expect(seen).toEqual(['finished x.wav'])
  })
})
