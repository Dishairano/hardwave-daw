/** Small colour helpers for canvas drawing. Colours are #rrggbb. */

/** Blend two colours; t is the share of b (0..1). Returns a unless both parse. */
export function mixHex(a: string, b: string, t: number): string {
  const pa = parseInt(a.replace('#', '').slice(0, 6), 16)
  const pb = parseInt(b.replace('#', '').slice(0, 6), 16)
  if (Number.isNaN(pa) || Number.isNaN(pb)) return a
  const ch = (v: number, s: number) => (v >> s) & 255
  const m = (s: number) => Math.round(ch(pa, s) * (1 - t) + ch(pb, s) * t)
  return '#' + [16, 8, 0].map((s) => m(s).toString(16).padStart(2, '0')).join('')
}

/** Perceived brightness 0..1 of a colour. */
export function hexLuma(c: string): number {
  const v = parseInt(c.replace('#', '').slice(0, 6), 16)
  if (Number.isNaN(v)) return 0
  return (0.2126 * ((v >> 16) & 255) + 0.7152 * ((v >> 8) & 255) + 0.0722 * (v & 255)) / 255
}
