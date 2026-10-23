export interface SectionSpan {
  id: string
  name: string
  startTicks: number
  endTicks: number
}

/** The section under a tick on the ruler, the latest-starting one when
 *  sections overlap. */
export function sectionAt(sections: SectionSpan[], tick: number): SectionSpan | null {
  let hit: SectionSpan | null = null
  for (const s of sections) {
    if (tick >= s.startTicks && tick < s.endTicks && (!hit || s.startTicks >= hit.startTicks)) hit = s
  }
  return hit
}

/**
 * What `move_section` needs to swap a section with its neighbour.
 *
 * The command puts a section before another one, or at the end when
 * `before` is null. Earlier is "before the previous section"; later is
 * "before the section after the next one", or the end when the next one is
 * the last. Undefined means there is no neighbour on that side.
 */
export function sectionMoves(
  sections: SectionSpan[],
  id: string,
): { earlier: string | undefined; later: string | null | undefined } {
  const sorted = [...sections].sort((a, b) => a.startTicks - b.startTicks)
  const i = sorted.findIndex(s => s.id === id)
  if (i < 0) return { earlier: undefined, later: undefined }
  const earlier = i > 0 ? sorted[i - 1].id : undefined
  const later = i < sorted.length - 1 ? (sorted[i + 2]?.id ?? null) : undefined
  return { earlier, later }
}
