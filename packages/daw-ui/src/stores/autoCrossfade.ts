/**
 * Auto-crossfade: where two audio clips of one channel overlap on a row,
 * the earlier one fades out over the overlap and the later one fades in,
 * so they blend instead of playing on top of each other. The playlist
 * shades those overlaps (Arrangement.tsx); this works out the fades.
 */

export interface FadeClip {
  id: string
  kind: string
  position_ticks: number
  length_ticks: number
  fadeInTicks: number
  fadeOutTicks: number
  row?: number | null
}

export interface FadeEdit {
  clipId: string
  fadeInTicks: number
  fadeOutTicks: number
}

/** The fades for every overlap between one channel's audio clips on the
 *  same row, one edit per clip that changes. A clip between two others
 *  gets both its fades; a fade with no overlap stays as it was. */
export function overlapFades(clips: FadeClip[]): { edits: FadeEdit[]; overlaps: number } {
  const byRow = new Map<number, FadeClip[]>()
  for (const c of clips) {
    if (c.kind !== 'audio') continue
    const row = c.row ?? -1
    let list = byRow.get(row)
    if (!list) byRow.set(row, (list = []))
    list.push(c)
  }
  const fades = new Map<string, FadeEdit>()
  const fadeOf = (c: FadeClip) => {
    let f = fades.get(c.id)
    if (!f) fades.set(c.id, (f = { clipId: c.id, fadeInTicks: c.fadeInTicks, fadeOutTicks: c.fadeOutTicks }))
    return f
  }
  let overlaps = 0
  for (const list of byRow.values()) {
    list.sort((a, b) => a.position_ticks - b.position_ticks)
    for (let j = 1; j < list.length; j++) {
      const prev = list[j - 1]
      const cur = list[j]
      const overlap = prev.position_ticks + prev.length_ticks - cur.position_ticks
      if (overlap <= 0) continue
      fadeOf(prev).fadeOutTicks = Math.min(overlap, prev.length_ticks)
      fadeOf(cur).fadeInTicks = Math.min(overlap, cur.length_ticks)
      overlaps++
    }
  }
  return { edits: [...fades.values()], overlaps }
}
