import { create } from 'zustand'

/**
 * Global "Hover anything for live info" channel. Any control can describe
 * itself here; the top-bar status strip subscribes and shows it live.
 *
 * Most controls don't need to call this directly — a delegated mouseover
 * listener in HwApp reads the nearest element's `data-hint` (preferred) or
 * its `title` attribute and pushes it here, so every control that already
 * has a `title` feeds the info bar for free. Components that want a *live*
 * value (e.g. the current dB while dragging a fader) can call `setInfo`
 * imperatively and `clearInfo` when done.
 */
interface HoverInfoState {
  info: string
  /** Set the live info line. */
  setInfo: (info: string) => void
  /** Clear back to the idle/default line. */
  clearInfo: () => void
}

export const useHoverInfoStore = create<HoverInfoState>((set) => ({
  info: '',
  setInfo: (info) => set({ info }),
  clearInfo: () => set({ info: '' }),
}))
