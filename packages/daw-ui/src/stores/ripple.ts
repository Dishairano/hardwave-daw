/** A clip on a track, as far as closing gaps is concerned. */
export interface Placed {
  id: string
  position_ticks: number
  length_ticks: number
}

/**
 * Where the clips that stay end up when `deleted` go and the gaps close
 * (ripple delete), on one track. A deleted clip's space only closes where no
 * remaining clip covers it, so closing never pushes clips onto each other.
 * Returns the clips that move, with their new positions.
 */
export function rippleShifts(clips: Placed[], deleted: Set<string>): Map<string, number> {
  const staying = clips.filter(c => !deleted.has(c.id))
  // The deleted ranges, merged where they touch or overlap.
  const ranges = clips
    .filter(c => deleted.has(c.id))
    .map(c => [c.position_ticks, c.position_ticks + c.length_ticks] as [number, number])
    .sort((a, b) => a[0] - b[0])
  const merged: [number, number][] = []
  for (const r of ranges) {
    const last = merged[merged.length - 1]
    if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1])
    else merged.push([r[0], r[1]])
  }
  // A gap is a merged range no staying clip reaches into.
  const gaps = merged.filter(([a, b]) =>
    !staying.some(c => c.position_ticks < b && c.position_ticks + c.length_ticks > a))
  const moved = new Map<string, number>()
  for (const c of staying) {
    const shift = gaps
      .filter(([, b]) => b <= c.position_ticks)
      .reduce((sum, [a, b]) => sum + (b - a), 0)
    if (shift > 0) moved.set(c.id, c.position_ticks - shift)
  }
  return moved
}
