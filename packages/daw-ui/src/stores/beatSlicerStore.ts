import { create } from 'zustand'

interface BeatSlicerState {
  openPath: string | null
  open: (path: string) => void
  close: () => void
}

export const useBeatSlicerStore = create<BeatSlicerState>((set) => ({
  openPath: null,
  open: (path) => set({ openPath: path }),
  close: () => set({ openPath: null }),
}))
