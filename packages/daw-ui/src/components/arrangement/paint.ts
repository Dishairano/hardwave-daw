/**
 * Grid maths for the playlist's paint tool.
 *
 * Kept out of the canvas component so the two decisions that make painting
 * feel right can be tested: which slot the pointer is over, and whether that
 * slot is already taken.
 */
import type { ClipInfo } from '../../stores/trackStore'

/**
 * The tick of the grid slot under a pointer position.
 *
 * Floors to the slot rather than rounding to the nearest line, which is what
 * the rest of the playlist does for a single paste. Rounding while painting
 * puts the first copy of a drag half a slot behind the cursor, and the visible
 * result is a clip that appears to the left of where the drag started.
 */
export function paintSlotTick(
  mouseX: number,
  scrollOffset: number,
  pixelsPerTick: number,
  stepTicks: number,
): number {
  if (!(pixelsPerTick > 0) || stepTicks <= 0) return 0
  const tick = (mouseX + scrollOffset) / pixelsPerTick
  return Math.max(0, Math.floor(tick / stepTicks) * stepTicks)
}

/**
 * True when a clip already covers any part of the slot.
 *
 * Painting on top of an existing clip would bury it under a copy, which reads
 * as a clip that can no longer be selected.
 */
export function paintSlotOccupied(
  clips: ClipInfo[] | undefined,
  tick: number,
  lengthTicks: number,
): boolean {
  if (!clips || clips.length === 0) return false
  return clips.some(
    c => c.position_ticks < tick + lengthTicks && c.position_ticks + c.length_ticks > tick,
  )
}
