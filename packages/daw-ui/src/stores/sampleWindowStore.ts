import { create } from 'zustand'

/** Which clip's sample window is open: the clip that was double-clicked.
 *  Its settings are the sample's, and reach every clip of that sample. */
interface SampleWindowState {
  target: { trackId: string; clipId: string } | null
  open: (trackId: string, clipId: string) => void
  close: () => void
}

export const useSampleWindowStore = create<SampleWindowState>((set) => ({
  target: null,
  open: (trackId, clipId) => set({ target: { trackId, clipId } }),
  close: () => set({ target: null }),
}))
