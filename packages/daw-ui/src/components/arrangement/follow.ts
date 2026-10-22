/**
 * Where the playlist scrolls to while following the playhead.
 *
 * `continuous` keeps the playhead a quarter of the way in, so the view slides
 * under it. `page` leaves the view still until the playhead nears the right
 * edge, then turns the page, as FL Studio's page follow does: the clips stay
 * readable while they play.
 */
export type FollowMode = 'continuous' | 'page'

export function followScroll(playheadPx: number, width: number, mode: FollowMode): number {
  if (width <= 0) return 0
  if (mode === 'page') {
    // A page is most of the width, so the playhead lands a little in from
    // the left edge after the turn and the last bit of the old page stays
    // in view.
    const page = width * 0.9
    return Math.max(0, Math.floor(Math.max(0, playheadPx) / page) * page)
  }
  return Math.max(0, playheadPx - width * 0.25)
}
