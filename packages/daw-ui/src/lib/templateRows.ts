/**
 * Where a built-in project template puts its tracks.
 *
 * A new project's playlist and mixer are its numbered insert rows
 * (insert-001 to insert-500). A template used to add its tracks after
 * them, five hundred rows down, where nobody saw them: a new producer who
 * picked "Vocal Session" found an empty playlist and mixer. The template
 * now names the first rows instead, as the starter songs do.
 */

/** The rows a template names, from the first one down. */
export const TEMPLATE_ROWS: Record<'vocal' | 'mixing', string[]> = {
  vocal: ['Vocal', 'Backing Vocals', 'FX Return'],
  mixing: [
    'Track 1', 'Track 2', 'Track 3', 'Track 4',
    'Track 5', 'Track 6', 'Track 7', 'Track 8',
    'Bus A', 'Bus B',
  ],
}

/** The id of playlist row `row`, counting from one, in a new project. */
export function insertRowId(row: number): string {
  return `insert-${String(row).padStart(3, '0')}`
}

/**
 * The track each of `names` goes on: the first rows of the playlist that
 * are there. A song without the numbered rows (one made before them)
 * gets fewer, and the caller adds tracks for the rest.
 */
export function templateRowTargets(
  tracks: { id: string }[],
  names: string[],
): { trackId: string; name: string }[] {
  const present = new Set(tracks.map((t) => t.id))
  const out: { trackId: string; name: string }[] = []
  for (let i = 0; i < names.length; i++) {
    const trackId = insertRowId(i + 1)
    if (!present.has(trackId)) break
    out.push({ trackId, name: names[i] })
  }
  return out
}
