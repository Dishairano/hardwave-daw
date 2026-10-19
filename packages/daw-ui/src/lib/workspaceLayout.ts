/**
 * Where the panels sit in the work area: free windows that snap to the
 * edges and to each other, dock against an edge, maximize, and come in a
 * few ready layouts. The approved mockup is
 * suite.hardwavestudios.com/daw-panels-mockup/. Pure maths, so the frame
 * component only wires pointer events to these.
 */

export type PanelKey = 'browser' | 'channelRack' | 'pianoRoll' | 'playlist' | 'mixer'
export type DockSide = 'left' | 'right' | 'top' | 'bottom'
export type LayoutName = 'default' | 'arrange' | 'mix' | 'free' | 'mine'

export interface Rect { x: number; y: number; w: number; h: number }
export interface Area { w: number; h: number }

export interface PanelGeometry extends Rect {
  /** The edge it is docked against, if any. */
  dock: DockSide | null
  /** Fills the work area until restored. */
  max: boolean
  /** Stacking order: higher is in front. */
  z: number
}

export const PANEL_KEYS: PanelKey[] = ['browser', 'channelRack', 'playlist', 'pianoRoll', 'mixer']

/** The smallest a panel can be made, per panel. */
export const MIN_SIZE: Record<PanelKey, [number, number]> = {
  browser: [200, 160],
  channelRack: [340, 150],
  pianoRoll: [340, 180],
  playlist: [360, 180],
  mixer: [300, 180],
}

/** How close an edge has to come to another before it snaps, in pixels. */
export const SNAP = 9
/** How close the pointer has to come to a work-area edge to dock there. */
export const DOCK_EDGE = 26

type Share = [x: number, y: number, w: number, h: number, dock?: DockSide]

/**
 * The ready layouts, in shares of the work area. A panel left out of a
 * layout keeps its own place and is only shown when asked for.
 */
export const PRESETS: Record<Exclude<LayoutName, 'mine'>, Partial<Record<PanelKey, Share>>> = {
  default: { browser: [0, 0, 0.19, 1, 'left'], channelRack: [0.21, 0.03, 0.36, 0.34], mixer: [0.59, 0.03, 0.39, 0.34], playlist: [0.19, 0.4, 0.81, 0.6, 'bottom'] },
  arrange: { browser: [0, 0, 0.17, 1, 'left'], playlist: [0.17, 0, 0.83, 0.68, 'top'], pianoRoll: [0.17, 0.68, 0.83, 0.32, 'bottom'] },
  mix: { browser: [0, 0, 0.17, 1, 'left'], playlist: [0.17, 0, 0.83, 0.42, 'top'], mixer: [0.17, 0.42, 0.83, 0.58, 'bottom'] },
  free: { browser: [0.02, 0.05, 0.2, 0.6], channelRack: [0.25, 0.04, 0.34, 0.3], playlist: [0.3, 0.38, 0.56, 0.5], pianoRoll: [0.62, 0.06, 0.34, 0.36], mixer: [0.05, 0.69, 0.4, 0.28] },
}

/** Which panels a ready layout shows. */
export function presetShows(name: Exclude<LayoutName, 'mine'>): PanelKey[] {
  return PANEL_KEYS.filter((k) => PRESETS[name][k])
}

/** Every panel's place for a ready layout in this work area. */
export function presetGeometry(
  name: Exclude<LayoutName, 'mine'>,
  area: Area,
  previous: Partial<Record<PanelKey, PanelGeometry>> = {},
): Record<PanelKey, PanelGeometry> {
  const out = {} as Record<PanelKey, PanelGeometry>
  let z = 1
  for (const key of PANEL_KEYS) {
    const s = PRESETS[name][key]
    if (s) {
      out[key] = {
        x: Math.round(s[0] * area.w), y: Math.round(s[1] * area.h),
        w: Math.round(s[2] * area.w), h: Math.round(s[3] * area.h),
        dock: s[4] ?? null, max: false, z: z++,
      }
    } else {
      const p = previous[key]
      out[key] = p
        ? { ...p, dock: null, max: false, z: z++ }
        : { x: Math.round(area.w * 0.25), y: Math.round(area.h * 0.15), w: Math.round(area.w * 0.5), h: Math.round(area.h * 0.5), dock: null, max: false, z: z++ }
    }
  }
  return out
}

/** Keep a panel at least its minimum size, no larger than the work area,
 *  and with enough of its title bar inside to grab it again. */
export function clampRect(r: Rect, key: PanelKey, area: Area): Rect {
  const [minW, minH] = MIN_SIZE[key]
  const w = Math.max(Math.min(minW, area.w), Math.min(r.w, area.w))
  const h = Math.max(Math.min(minH, area.h), Math.min(r.h, area.h))
  const x = Math.max(0, Math.min(r.x, area.w - Math.min(w, 80)))
  const y = Math.max(0, Math.min(r.y, area.h - 28))
  return { x, y, w, h }
}

/** A line to draw while snapping: vertical at x, or horizontal at y. */
export type Guide = { axis: 'x'; at: number } | { axis: 'y'; at: number }

function edgesOf(area: Area, others: Rect[]): { xs: number[]; ys: number[] } {
  return {
    xs: [0, area.w, ...others.flatMap((o) => [o.x, o.x + o.w])],
    ys: [0, area.h, ...others.flatMap((o) => [o.y, o.y + o.h])],
  }
}

/** The nearest target within SNAP of any of the values, as the shift to it. */
function nearest(values: number[], targets: number[]): { shift: number; at: number } | null {
  let best: { shift: number; at: number } | null = null
  for (const t of targets) {
    for (const v of values) {
      const shift = t - v
      if (Math.abs(shift) <= SNAP && (!best || Math.abs(shift) < Math.abs(best.shift))) best = { shift, at: t }
    }
  }
  return best
}

/** A panel moved to `r`: its edges pulled onto the work area's edges and
 *  the other panels' edges when close. */
export function snapMove(r: Rect, others: Rect[], area: Area): { rect: Rect; guides: Guide[] } {
  const { xs, ys } = edgesOf(area, others)
  const guides: Guide[] = []
  let { x, y } = r
  const sx = nearest([x, x + r.w], xs)
  if (sx) { x += sx.shift; guides.push({ axis: 'x', at: sx.at }) }
  const sy = nearest([y, y + r.h], ys)
  if (sy) { y += sy.shift; guides.push({ axis: 'y', at: sy.at }) }
  return { rect: { ...r, x, y }, guides }
}

/** Which way a resize handle pulls: any of n, s, e, w. */
export type ResizeDir = 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw'

/** A panel resized from `start` by a handle dragged (dx, dy): the moving
 *  edges snap like a move, and it never goes under its minimum size. */
export function snapResize(
  start: Rect, dir: ResizeDir, dx: number, dy: number,
  key: PanelKey, others: Rect[], area: Area,
): { rect: Rect; guides: Guide[] } {
  const { xs, ys } = edgesOf(area, others)
  const [minW, minH] = MIN_SIZE[key]
  const guides: Guide[] = []
  let { x, y, w, h } = start
  if (dir.includes('e')) {
    w = start.w + dx
    const s = nearest([x + w], xs)
    if (s) { w += s.shift; guides.push({ axis: 'x', at: s.at }) }
  }
  if (dir.includes('w')) {
    x = start.x + dx; w = start.w - dx
    const s = nearest([x], xs)
    if (s) { x += s.shift; w -= s.shift; guides.push({ axis: 'x', at: s.at }) }
  }
  if (dir.includes('s')) {
    h = start.h + dy
    const s = nearest([y + h], ys)
    if (s) { h += s.shift; guides.push({ axis: 'y', at: s.at }) }
  }
  if (dir.includes('n')) {
    y = start.y + dy; h = start.h - dy
    const s = nearest([y], ys)
    if (s) { y += s.shift; h -= s.shift; guides.push({ axis: 'y', at: s.at }) }
  }
  if (w < minW) { if (dir.includes('w')) x -= minW - w; w = minW }
  if (h < minH) { if (dir.includes('n')) y -= minH - h; h = minH }
  return { rect: { x: Math.round(x), y: Math.round(y), w: Math.round(w), h: Math.round(h) }, guides }
}

/** The edge a dragged panel would dock against, from where the pointer is. */
export function dockZone(px: number, py: number, area: Area): DockSide | null {
  if (px < DOCK_EDGE) return 'left'
  if (px > area.w - DOCK_EDGE) return 'right'
  if (py < DOCK_EDGE) return 'top'
  if (py > area.h - DOCK_EDGE) return 'bottom'
  return null
}

/** Where a panel of this size goes when docked against `side`: the whole
 *  side, at most 45 % of the width or 55 % of the height. */
export function dockRect(side: DockSide, size: { w: number; h: number }, area: Area): Rect {
  const w = Math.round(Math.min(size.w, area.w * 0.45))
  const h = Math.round(Math.min(size.h, area.h * 0.55))
  switch (side) {
    case 'left': return { x: 0, y: 0, w, h: area.h }
    case 'right': return { x: area.w - w, y: 0, w, h: area.h }
    case 'top': return { x: 0, y: 0, w: area.w, h }
    case 'bottom': return { x: 0, y: area.h - h, w: area.w, h }
  }
}

/** The work area changed size: docked panels keep their side and span,
 *  free ones keep their place (clamped back in). */
export function fitToArea(g: PanelGeometry, key: PanelKey, from: Area, to: Area): PanelGeometry {
  let r: Rect = { x: g.x, y: g.y, w: g.w, h: g.h }
  if (g.dock === 'left' || g.dock === 'right') {
    r = { ...r, y: 0, h: to.h, x: g.dock === 'right' ? to.w - r.w : 0 }
  } else if (g.dock === 'top' || g.dock === 'bottom') {
    // A docked bar that started part way across scales with the area.
    const sx = from.w > 0 ? to.w / from.w : 1
    r = { ...r, x: Math.round(r.x * sx), w: Math.round(r.w * sx), y: g.dock === 'bottom' ? to.h - r.h : 0 }
  }
  return { ...g, ...clampRect(r, key, to) }
}
