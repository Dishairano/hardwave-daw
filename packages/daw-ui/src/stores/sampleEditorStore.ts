import { create } from 'zustand'

interface SampleEditorState {
  openPath: string | null
  open: (path: string) => void
  close: () => void
}

export const useSampleEditorStore = create<SampleEditorState>((set) => ({
  openPath: null,
  open: (path) => set({ openPath: path }),
  close: () => set({ openPath: null }),
}))
