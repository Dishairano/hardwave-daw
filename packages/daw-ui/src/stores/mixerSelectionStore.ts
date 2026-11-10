import { create } from 'zustand'

/**
 * Single source of truth for "which mixer strip is currently selected".
 *
 * The FxRackPanel (right column) subscribes to this so it can retarget
 * itself when the user clicks a different strip — without having to
 * prop-drill the selection through every ChannelStrip. Strip-side click
 * handlers also use `selectStrip(id)` rather than carrying a callback
 * via every render path.
 *
 * Selection is intentionally a separate store from `trackStore`. Track
 * state changes far more often than the selection (every meter tick,
 * every fader pixel) — keeping selection in its own store means
 * "selected" subscribers don't re-render on every track field update.
 *
 * `selectedTrackId === null` means "no strip selected" — FxRackPanel
 * shows an empty state. The MixerPanel seeds the store with the master
 * track id on first mount so the rack always has a sensible default.
 */
interface MixerSelectionState {
  selectedTrackId: string | null
  /** An empty insert number picked in the mixer (no track behind it yet),
   *  so an effect can be put on it; null while a track is selected. */
  selectedEmptyNumber: number | null
  selectTrack: (id: string | null) => void
  selectEmptyNumber: (n: number) => void
  /** The insert number whose name is being edited on its strip. */
  renamingNumber: number | null
  setRenamingNumber: (n: number | null) => void
  /** A strip's right-click menu: its number and where it opened. */
  stripMenu: { number: number; x: number; y: number } | null
  openStripMenu: (menu: { number: number; x: number; y: number } | null) => void
}

export const useMixerSelectionStore = create<MixerSelectionState>((set) => ({
  selectedTrackId: null,
  selectedEmptyNumber: null,
  selectTrack: (id) => set({ selectedTrackId: id, selectedEmptyNumber: null }),
  selectEmptyNumber: (n) => set({ selectedTrackId: null, selectedEmptyNumber: n }),
  renamingNumber: null,
  setRenamingNumber: (n) => set({ renamingNumber: n, stripMenu: null }),
  stripMenu: null,
  openStripMenu: (menu) => set({ stripMenu: menu }),
}))
