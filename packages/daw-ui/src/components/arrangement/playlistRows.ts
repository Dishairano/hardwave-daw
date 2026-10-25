/**
 * Which tracks the playlist shows, and on which row each one sits.
 *
 * The grid and the name column beside it each had their own answer: the
 * column listed only the numbered inserts and gave every automation lane a
 * row of its own, while the grid drew every track and no lane rows. Once
 * the two disagreed, every row below the first difference was off. Both
 * now read the layout from here.
 */

export interface PlaylistTrackLike {
  id: string
  kind: string
  automationLanes: unknown[]
  automationClips?: unknown[]
}

/** Tracks shown in the playlist: everything but the master and tracks in a
 *  collapsed folder, in project order. */
export function playlistTracks<T extends PlaylistTrackLike>(
  tracks: T[],
  folders: { collapsed: boolean; trackIds: string[] }[],
): T[] {
  const hidden = new Set<string>()
  for (const f of folders) if (f.collapsed) for (const id of f.trackIds) hidden.add(id)
  return tracks.filter((t) => t.kind !== 'Master' && !hidden.has(t.id))
}

export interface RowLayout {
  /** The row each track's clips sit on. */
  rowOf: number[]
  /** For every row, the track it belongs to (its own row or one of its
   *  automation rows). */
  ownerOf: number[]
  /** For every row, whether it is a track's own row (false: automation). */
  isTrackRow: boolean[]
  /** Rows in all. */
  total: number
}

/** A track takes one row, then one per automation lane and automation clip
 *  under it, all the same height. */
export function playlistRowLayout(tracks: PlaylistTrackLike[]): RowLayout {
  const rowOf: number[] = []
  const ownerOf: number[] = []
  const isTrackRow: boolean[] = []
  tracks.forEach((t, i) => {
    rowOf.push(ownerOf.length)
    ownerOf.push(i)
    isTrackRow.push(true)
    const extra = t.automationLanes.length + (t.automationClips?.length ?? 0)
    for (let k = 0; k < extra; k++) {
      ownerOf.push(i)
      isTrackRow.push(false)
    }
  })
  return { rowOf, ownerOf, isTrackRow, total: ownerOf.length }
}

/** One row of the playlist's name column. */
export type ColumnRow<T extends PlaylistTrackLike> =
  | { kind: 'track'; track: T }
  | { kind: 'lane'; track: T; lane: T['automationLanes'][number] }
  | { kind: 'clip'; track: T; clip: NonNullable<T['automationClips']>[number] }
  | { kind: 'empty'; slot: number }

/** The name column's rows in order, the same rows `playlistRowLayout`
 *  counts: each track, its automation lanes and clips, then the numbered
 *  empty slots that fill the grid out. */
export function playlistColumnRows<T extends PlaylistTrackLike>(
  tracks: T[],
  emptySlots: number,
): ColumnRow<T>[] {
  const rows: ColumnRow<T>[] = []
  for (const track of tracks) {
    rows.push({ kind: 'track', track })
    for (const lane of track.automationLanes) rows.push({ kind: 'lane', track, lane })
    for (const clip of track.automationClips ?? []) rows.push({ kind: 'clip', track, clip })
  }
  for (let i = 0; i < emptySlots; i++) rows.push({ kind: 'empty', slot: tracks.length + i + 1 })
  return rows
}

/** Rows to draw for a view `viewHeight` tall scrolled `scrollY` down, with a
 *  few to spare either side so a fast scroll does not show a gap. */
export function visibleRows(
  count: number,
  rowHeight: number,
  scrollY: number,
  viewHeight: number,
  spare = 4,
): { first: number; last: number } {
  const h = Math.max(1, rowHeight)
  const first = Math.max(0, Math.floor(scrollY / h) - spare)
  const last = Math.min(count, Math.ceil((scrollY + viewHeight) / h) + spare)
  return { first: Math.min(first, last), last }
}
