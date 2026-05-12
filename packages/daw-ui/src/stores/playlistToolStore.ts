import { create } from 'zustand'

/**
 * FL Studio-style playlist tool selection.
 *
 * Replaces the implicit "left-click is always paste, hold is select"
 * model from v0.159.2 with an explicit tool-mode store. The default
 * tool stays `draw` so legacy behaviour is preserved; switching tools
 * via keybind or icon click changes what a left-click means.
 *
 *   draw    — paint a single clip at the click position (FL default).
 *             Hold + drag promotes to marquee selection at >4px threshold.
 *   paint   — drag-paint multiple clips along the path.
 *   slice   — click on a clip to split it at the snap-aligned tick.
 *   delete  — click on a clip to remove it.
 *   mute    — click on a clip to toggle its muted flag.
 *   slip    — click + drag inside a clip to shift its content while
 *             keeping the clip bounds fixed (changes audio start offset).
 *   select  — every left-click is rubber-band; no draw/paint side-effect.
 *   zoom    — click to zoom in, alt-click to zoom out.
 *
 * Behaviour wiring lives in `Arrangement.tsx` mousedown branches.
 * Keybinds wire through `useShortcutsStore.matchEvent` in `App.tsx`.
 */

export type PlaylistTool =
  | 'draw'
  | 'paint'
  | 'slice'
  | 'delete'
  | 'mute'
  | 'slip'
  | 'select'
  | 'zoom'

interface PlaylistToolState {
  tool: PlaylistTool
  setTool: (t: PlaylistTool) => void
}

export const usePlaylistToolStore = create<PlaylistToolState>((set) => ({
  tool: 'draw',
  setTool: (tool) => set({ tool }),
}))
