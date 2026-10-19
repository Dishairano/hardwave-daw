/**
 * How far one wheel movement zooms the playlist.
 *
 * Up to the old top of the range (16, 1600 %) a mouse notch is about 18 %,
 * as it was, so ordinary editing keeps its feel. Past it the steps grow
 * with the zoom, so the samples (4000, 400000 %) are about a dozen notches
 * further rather than two dozen: with fixed steps, getting from a song to
 * its samples took some fifty notches.
 */
const PER_PIXEL = 0.0014
const ACCEL_FROM = 16
const ACCEL = 0.6

/** The factor to multiply the zoom by for a wheel movement of `dy` pixels
 *  (negative zooms in) at zoom `zoom`. */
export function zoomStep(dy: number, zoom: number): number {
  const beyond = Math.max(0, Math.log2(Math.max(zoom, 1e-6) / ACCEL_FROM))
  return Math.exp(-dy * PER_PIXEL * (1 + ACCEL * beyond))
}
