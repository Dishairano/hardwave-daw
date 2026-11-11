/**
 * Which rows of the playlist are drawn: only those in view. The rows
 * themselves and what sits on them are in `rowIndex.ts`.
 */

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
