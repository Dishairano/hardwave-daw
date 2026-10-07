import { create } from 'zustand'

/**
 * How far down the playlist is scrolled, shared by the grid and the track
 * name column beside it. The grid kept this to itself, so scrolling moved
 * the clips and left the names where they were.
 */
interface PlaylistScrollState {
  /** Pixels scrolled down from the first row. */
  y: number
  /** The furthest it can go: content height minus what fits on screen. */
  max: number
  setY: (y: number) => void
  setMax: (max: number) => void
  /** Scroll by a wheel step, kept inside 0..max. */
  scrollBy: (dy: number) => void
}

const clamp = (v: number, max: number) => Math.max(0, Math.min(max, v))

export const usePlaylistScrollStore = create<PlaylistScrollState>((set) => ({
  y: 0,
  max: 0,
  setY: (y) => set({ y: Math.max(0, y) }),
  setMax: (max) => set((s) => ({ max, y: clamp(s.y, max) })),
  scrollBy: (dy) => set((s) => ({ y: clamp(s.y + dy, s.max) })),
}))
