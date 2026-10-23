/**
 * Snapping to what is already in the song: other clips' edges and markers.
 *
 * Grid snap alone made lining a clip up with its neighbour a hunt when the
 * neighbour sat off the grid (a recorded take, a nudged hit). Within a few
 * pixels of an edge or a marker, the clip's start or end sticks to it, as
 * FL Studio's magnetic snap does; further away the grid decides.
 */

/**
 * Where a clip `length` ticks long starts when dragged to `raw`: on a target
 * (by its start or its end) when one is within `reach` ticks, else on
 * `gridded`, the grid's answer.
 */
export function magnetSnap(raw: number, length: number, gridded: number, targets: number[], reach: number): number {
  let best = gridded
  let bestDist = reach
  for (const t of targets) {
    for (const start of [t, t - length]) {
      const d = Math.abs(raw - start)
      if (start >= 0 && d < bestDist) {
        best = start
        bestDist = d
      }
    }
  }
  return best
}

/** Where a single edge dragged to `raw` lands: on a target within reach, else on the grid. */
export function magnetEdge(raw: number, gridded: number, targets: number[], reach: number): number {
  return magnetSnap(raw, 0, gridded, targets, reach)
}
