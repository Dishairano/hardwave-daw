import { create } from 'zustand'

/**
 * What the sample window is open on: an audio clip (its settings are the
 * sample's, and reach every clip of that sample) or a sampler channel (the
 * channel's own sampler, by its slot).
 */
export type SampleWindowTarget =
  | { kind: 'clip'; trackId: string; clipId: string }
  | { kind: 'sampler'; trackId: string; slotId: string }

interface SampleWindowState {
  target: SampleWindowTarget | null
  open: (trackId: string, clipId: string) => void
  openSampler: (trackId: string, slotId: string) => void
  close: () => void
}

export const useSampleWindowStore = create<SampleWindowState>((set) => ({
  target: null,
  open: (trackId, clipId) => set({ target: { kind: 'clip', trackId, clipId } }),
  openSampler: (trackId, slotId) => set({ target: { kind: 'sampler', trackId, slotId } }),
  close: () => set({ target: null }),
}))
