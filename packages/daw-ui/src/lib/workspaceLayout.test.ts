import { describe, expect, it } from 'vitest'
import {
  clampRect, dockRect, dockZone, fitToArea, MIN_SIZE, presetGeometry, presetShows, snapMove, snapResize,
} from './workspaceLayout'

const area = { w: 1400, h: 800 }

describe('ready layouts', () => {
  it('place the panels they show in shares of the work area', () => {
    const g = presetGeometry('default', area)
    expect(g.browser).toMatchObject({ x: 0, y: 0, w: 266, h: 800, dock: 'left' })
    expect(g.playlist.dock).toBe('bottom')
    expect(presetShows('default')).toEqual(['browser', 'channelRack', 'playlist', 'mixer'])
    expect(presetShows('mix')).toContain('mixer')
  })

  it('leave a panel they do not show where it was', () => {
    const before = presetGeometry('free', area)
    const g = presetGeometry('default', area, before)
    expect(g.pianoRoll).toMatchObject({ x: before.pianoRoll.x, y: before.pianoRoll.y, dock: null })
  })
})

describe('moving', () => {
  it('snaps an edge onto a nearby panel edge and the work area edge', () => {
    const other = { x: 300, y: 100, w: 200, h: 200 }
    const { rect, guides } = snapMove({ x: 506, y: 4, w: 300, h: 200 }, [other], area)
    expect(rect).toMatchObject({ x: 500, y: 0 })
    expect(guides).toEqual([{ axis: 'x', at: 500 }, { axis: 'y', at: 0 }])
  })

  it('leaves a panel alone when nothing is close', () => {
    const r = { x: 600, y: 300, w: 200, h: 150 }
    expect(snapMove(r, [], area)).toEqual({ rect: r, guides: [] })
  })
})

describe('resizing', () => {
  const start = { x: 100, y: 100, w: 400, h: 300 }

  it('moves only the dragged edges and snaps them', () => {
    const { rect } = snapResize(start, 'se', 895, 396, 'mixer', [], area)
    expect(rect).toEqual({ x: 100, y: 100, w: 1300, h: 700 })
  })

  it('never goes under the minimum size, and a left edge stays put at it', () => {
    const { rect } = snapResize(start, 'nw', 390, 290, 'mixer', [], area)
    const [minW, minH] = MIN_SIZE.mixer
    expect(rect).toEqual({ x: 100 + 400 - minW, y: 100 + 300 - minH, w: minW, h: minH })
  })
})

describe('docking', () => {
  it('docks when the pointer reaches an edge', () => {
    expect(dockZone(5, 400, area)).toBe('left')
    expect(dockZone(1390, 400, area)).toBe('right')
    expect(dockZone(700, 790, area)).toBe('bottom')
    expect(dockZone(700, 400, area)).toBeNull()
  })

  it('takes the whole side, no more than part of the area', () => {
    expect(dockRect('left', { w: 900, h: 300 }, area)).toEqual({ x: 0, y: 0, w: 630, h: 800 })
    expect(dockRect('bottom', { w: 300, h: 300 }, area)).toEqual({ x: 0, y: 500, w: 1400, h: 300 })
  })
})

describe('the work area changing size', () => {
  it('keeps docked panels on their side', () => {
    const g = { x: 1000, y: 0, w: 400, h: 800, dock: 'right' as const, max: false, z: 1 }
    expect(fitToArea(g, 'mixer', area, { w: 1000, h: 600 })).toMatchObject({ x: 600, y: 0, w: 400, h: 600 })
  })

  it('pulls a free panel back in when it would fall out', () => {
    expect(clampRect({ x: 1390, y: 790, w: 300, h: 200 }, 'mixer', area)).toEqual({ x: 1320, y: 772, w: 300, h: 200 })
  })
})
