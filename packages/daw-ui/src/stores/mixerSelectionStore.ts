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
  selectTrack: (id: string | null) => void
}

export const useMixerSelectionStore = create<MixerSelectionState>((set) => ({
  selectedTrackId: null,
  selectTrack: (id) => set({ selectedTrackId: id }),
}))
