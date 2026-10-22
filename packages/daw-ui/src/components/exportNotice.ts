/** What an export returns: the file, and how loud it came out. */
export interface ExportOutcome {
  path: string
  duration_secs: number
  loudness?: { peakDb: number; truePeakDb: number; lufsIntegrated: number | null }
}

const signed = (v: number) => (v > 0 ? '+' : '') + v.toFixed(1)

/**
 * The notice after an export: how long, in what format, and how loud the
 * file is. A true peak over 0 dB makes it a warning, since streaming
 * services and MP3 encoders turn that into distortion. For MP3 and Ogg the
 * numbers are of the audio that went into the encoder; encoding moves the
 * peaks a little.
 */
export function describeExport(result: ExportOutcome): { title: string; detail: string; over: boolean } {
  const ext = (result.path.split('.').pop() ?? '').toUpperCase()
  const format = ['WAV', 'FLAC', 'MP3', 'OGG'].includes(ext) ? ext : 'audio'
  const head = `Exported ${result.duration_secs.toFixed(1)} s to ${format}`
  const l = result.loudness
  if (!l) return { title: head, detail: result.path, over: false }
  const lufs = l.lufsIntegrated == null ? 'too short for LUFS' : `${l.lufsIntegrated.toFixed(1)} LUFS`
  const over = l.truePeakDb > 0
  const lossy = format === 'MP3' || format === 'OGG' ? ' (before encoding)' : ''
  const title = `${head}: ${lufs}, true peak ${signed(l.truePeakDb)} dBTP${lossy}`
  const warning = over ? ' The true peak is over 0 dB: a limiter ceiling of -1 dB on the master keeps it clean on streaming services.' : ''
  return { title, detail: result.path + warning, over }
}
