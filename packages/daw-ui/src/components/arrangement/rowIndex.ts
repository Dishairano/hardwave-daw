/**
 * The playlist's rows and what sits on them.
 *
 * A row is FL's playlist track: it holds clips of any channel and says
 * nothing about how they sound. Each clip keeps the row it was put on
 * (`ClipInfo.row`); this index turns the tracks' clip lists into one list
 * per row, sorted by time, so the grid draws and hit-tests only what is on
 * screen. A song of 3,166 clips with 472 on one track used to be walked
 * whole for every frame and every mouse move.
 *
 * A clip made by something not yet taught about rows has no row. It is
 * drawn on the row most of its channel's other clips sit on, else on its
 * track's place in the playlist (where a song built row by row put it),
 * and an automation clip or lane without one on the first empty row.
 * The engine's row mute and solo follow the same rule (`shown_rows` in
 * crates/hardwave-project/src/layout.rs), the app sends every clip with
 * the row that rule gives it, and both are tested on the same cases
 * (shownRows.cases.json), so what a muted row looks like is what it does.
 */

export interface RowClipLike {
  id: string
  position_ticks: number
  length_ticks: number
  row?: number | null
}

export interface RowAutomationClipLike {
  id: string
  startTick: number
  lengthTicks: number
  row?: number | null
}

export interface RowLaneLike {
  id: string
  row?: number | null
}

export interface RowTrackLike {
  id: string
  kind: string
  clips: RowClipLike[]
  automationLanes: RowLaneLike[]
  automationClips?: RowAutomationClipLike[]
}

/** One thing drawn on a row: a clip (audio or MIDI) or an automation clip. */
export type RowItem<T extends RowTrackLike> =
  | { kind: 'clip'; track: T; trackIndex: number; clip: T['clips'][number]; start: number; end: number; row: number }
  | {
    kind: 'auto'
    track: T
    trackIndex: number
    clip: NonNullable<T['automationClips']>[number]
    start: number
    end: number
    row: number
  }

export interface RowIndex<T extends RowTrackLike> {
  /** What is on each row, sorted by where it starts. Rows with nothing
   *  on them are not in the map. */
  byRow: Map<number, RowItem<T>[]>
  /** The longest item on each row, in ticks: anything reaching into a
   *  window starts no earlier than this before it. */
  longest: Map<number, number>
  /** Automation lanes by the row they are drawn on. */
  lanes: Map<number, { track: T; lane: T['automationLanes'][number] }[]>
  /** The row every clip, automation clip and lane is drawn on, by id. */
  rowOf: Map<string, number>
  /** Rows from the top that hold anything. */
  used: number
}

/** Each track's place in the playlist: its position among the tracks that
 *  are not the master, or -1 for the master. A song built row by row (a new
 *  song, a template, an FL import) has each track's clips on this row. */
export function trackPlaces(tracks: { kind: string }[]): number[] {
  let n = 0
  return tracks.map((t) => (t.kind === 'Master' ? -1 : n++))
}

/** The row most of these sit on, the lowest of equals; undefined if none
 *  has one. */
function mostUsedRow(clips: RowClipLike[]): number | undefined {
  const counts = new Map<number, number>()
  for (const c of clips) {
    if (c.row == null) continue
    counts.set(c.row, (counts.get(c.row) ?? 0) + 1)
  }
  let best: number | undefined
  let bestCount = 0
  for (const [row, count] of counts) {
    if (count > bestCount || (count === bestCount && best !== undefined && row < best)) {
      best = row
      bestCount = count
    }
  }
  return best
}

/** Index the tracks' clips by row. */
export function buildRowIndex<T extends RowTrackLike>(tracks: T[]): RowIndex<T> {
  const byRow = new Map<number, RowItem<T>[]>()
  const longest = new Map<number, number>()
  const lanes = new Map<number, { track: T; lane: T['automationLanes'][number] }[]>()
  const rowOf = new Map<string, number>()
  const occupied = new Set<number>()
  let used = 0
  const put = (item: RowItem<T>) => {
    let list = byRow.get(item.row)
    if (!list) byRow.set(item.row, (list = []))
    list.push(item)
    longest.set(item.row, Math.max(longest.get(item.row) ?? 0, item.end - item.start))
    rowOf.set(item.clip.id, item.row)
    occupied.add(item.row)
    used = Math.max(used, item.row + 1)
  }
  const putLane = (track: T, lane: T['automationLanes'][number], row: number) => {
    let list = lanes.get(row)
    if (!list) lanes.set(row, (list = []))
    list.push({ track, lane })
    rowOf.set(lane.id, row)
    occupied.add(row)
    used = Math.max(used, row + 1)
  }

  const places = trackPlaces(tracks)
  // What still needs the first empty row once everything with a place of
  // its own is in.
  const homeless: (() => void)[] = []
  let nextEmpty = 0
  const firstEmpty = () => {
    while (occupied.has(nextEmpty)) nextEmpty++
    return nextEmpty
  }

  tracks.forEach((track, trackIndex) => {
    const fallback = mostUsedRow(track.clips) ?? (places[trackIndex] >= 0 ? places[trackIndex] : undefined)
    for (const clip of track.clips) {
      const start = clip.position_ticks
      const end = start + clip.length_ticks
      const row = clip.row ?? fallback
      if (row == null) {
        homeless.push(() => put({ kind: 'clip', track, trackIndex, clip, start, end, row: firstEmpty() }))
      } else {
        put({ kind: 'clip', track, trackIndex, clip, start, end, row })
      }
    }
    for (const clip of track.automationClips ?? []) {
      const start = clip.startTick
      const end = start + clip.lengthTicks
      if (clip.row == null) {
        homeless.push(() => put({ kind: 'auto', track, trackIndex, clip, start, end, row: firstEmpty() }))
      } else {
        put({ kind: 'auto', track, trackIndex, clip, start, end, row: clip.row })
      }
    }
    for (const lane of track.automationLanes) {
      if (lane.row == null) homeless.push(() => putLane(track, lane, firstEmpty()))
      else putLane(track, lane, lane.row)
    }
  })
  for (const place of homeless) place()
  for (const list of byRow.values()) list.sort((a, b) => a.start - b.start)
  return { byRow, longest, lanes, rowOf, used }
}

/** How many rows the playlist shows: FL's 500, or more when the song has
 *  named or used rows past them. */
export function shownRowCount(index: { used: number }, namedRows: number, minimum = 500): number {
  return Math.max(minimum, index.used, namedRows)
}

/** The first item on a row that starts at or after `tick`. */
function firstStartingAt<T extends RowTrackLike>(list: RowItem<T>[], tick: number): number {
  let lo = 0
  let hi = list.length
  while (lo < hi) {
    const mid = (lo + hi) >> 1
    if (list[mid].start < tick) lo = mid + 1
    else hi = mid
  }
  return lo
}

/** What on `row` reaches into `[from, to]` (ticks), in drawing order. */
export function itemsInView<T extends RowTrackLike>(
  index: RowIndex<T>,
  row: number,
  from: number,
  to: number,
): RowItem<T>[] {
  const list = index.byRow.get(row)
  if (!list) return []
  const reach = index.longest.get(row) ?? 0
  const out: RowItem<T>[] = []
  for (let i = firstStartingAt(list, from - reach); i < list.length; i++) {
    const item = list[i]
    if (item.start > to) break
    if (item.end >= from) out.push(item)
  }
  return out
}

/** The item on `row` under `tick`, the one drawn on top (the last drawn),
 *  or undefined. `slack` widens every item by that many ticks at both ends,
 *  so a clip a pixel wide can still be grabbed. */
export function itemAt<T extends RowTrackLike>(
  index: RowIndex<T>,
  row: number,
  tick: number,
  slack = 0,
): RowItem<T> | undefined {
  const hits = itemsInView(index, row, tick - slack, tick + slack)
  for (let i = hits.length - 1; i >= 0; i--) {
    const item = hits[i]
    if (tick >= item.start - slack && tick <= item.end + slack) return item
  }
  return undefined
}

/** The row under a point `y` pixels from the top of the grid view, or -1
 *  above the first row. */
export function rowAtY(y: number, top: number, rowHeight: number, scrollY: number): number {
  if (y < top || rowHeight <= 0) return -1
  return Math.floor((y - top + scrollY) / rowHeight)
}

/** Every item on rows `[rowFrom, rowTo]` reaching into `[from, to]`: what a
 *  rubber band drawn over them picks. */
export function itemsInBox<T extends RowTrackLike>(
  index: RowIndex<T>,
  rowFrom: number,
  rowTo: number,
  from: number,
  to: number,
): RowItem<T>[] {
  const out: RowItem<T>[] = []
  for (let r = Math.max(0, rowFrom); r <= rowTo; r++) out.push(...itemsInView(index, r, from, to))
  return out
}

/** A row's name as FL shows it: its own, or "Track N" for one nobody named. */
export function rowName(rows: { name: string }[], row: number): string {
  const name = rows[row]?.name
  return name ? name : `Track ${row + 1}`
}

/** Which rows are silent: the muted ones, and while any row is soloed every
 *  row that is not. Rows past the list are on, unless something is soloed. */
export function silentRows(rows: { muted: boolean; soloed: boolean }[]): (row: number) => boolean {
  const anySolo = rows.some((r) => r.soloed)
  if (!anySolo && !rows.some((r) => r.muted)) return () => false
  return (row) => {
    const r = rows[row]
    if (!r) return anySolo
    return r.muted || (anySolo && !r.soloed)
  }
}

// One index per list of tracks: the grid and the name column beside it
// both read it, and the store makes a new list whenever a track changes.
const indexes = new WeakMap<object, RowIndex<RowTrackLike>>()

/** The row index of this list of tracks, built once however many views
 *  ask for it. */
export function rowIndexOf<T extends RowTrackLike>(tracks: T[]): RowIndex<T> {
  let index = indexes.get(tracks) as RowIndex<T> | undefined
  if (!index) {
    index = buildRowIndex(tracks)
    indexes.set(tracks, index as unknown as RowIndex<RowTrackLike>)
  }
  return index
}
