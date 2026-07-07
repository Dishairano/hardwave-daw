/**
 * Translate raw backend project-load errors into something a producer
 * can act on. The backend surfaces zstd/msgpack/io errors verbatim
 * ("unknown frame magic number"), which reads like a crash to users
 * (deep-research P1-10). Detail keeps the raw error for bug reports.
 */
export function friendlyLoadError(err: unknown): { message: string; hint: string } {
  const raw = String(err)
  const lower = raw.toLowerCase()

  if (lower.includes('no such file') || lower.includes('not found') || lower.includes('os error 2')) {
    return {
      message: 'The project file could not be found.',
      hint: 'It may have been moved, renamed, or deleted. If it lives on an external or cloud-synced drive, make sure that drive is connected.',
    }
  }
  if (lower.includes('permission denied') || lower.includes('os error 13')) {
    return {
      message: 'Hardwave does not have permission to read this file.',
      hint: 'Check the file is not read-only or locked by another program.',
    }
  }
  if (
    lower.includes('magic number') ||
    lower.includes('zstd') ||
    lower.includes('msgpack') ||
    lower.includes('rmp') ||
    lower.includes('invalid marker') ||
    lower.includes('unexpected end')
  ) {
    return {
      message: 'The project file is corrupted or not a Hardwave project.',
      hint: 'Try File → Revert to last backup, or open an autosave from the crash-recovery folder. If this file came from a newer Hardwave version, update the DAW first.',
    }
  }
  return {
    message: 'The project could not be opened.',
    hint: 'Try File → Revert to last backup. If this keeps happening, send the details below with a bug report (Help → Export diagnostics).',
  }
}
