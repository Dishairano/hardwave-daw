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

/**
 * The sends the Mixing template sets up between its tracks, given the
 * track ids in row order: each of the eight tracks sends to Bus A and
 * Bus B. The template said "Bus A/B return sends, pre-routed" and set up
 * none. They start silent (`SILENT_SEND_DB`), ready to be turned up, so
 * the mix sounds the same until one is.
 */
export function mixingTemplateSends(trackIds: string[]): { from: string; to: string }[] {
  const [busA, busB] = [trackIds[8], trackIds[9]]
  if (!busA || !busB) return []
  return trackIds.slice(0, 8).flatMap((from) => [{ from, to: busA }, { from, to: busB }])
}

/** The level a template's send starts at: as low as a send goes. */
export const SILENT_SEND_DB = -100
