/*
 * pickerStore — single selection across the FL-style picker panel.
 *
 * Click an entry in the picker → it becomes the active "paint" item.
 * Left-click anywhere on the playlist canvas (in a track row, not on
 * an existing clip) then places a copy of that item at the clicked
 * tick. This is the FL Studio Pattern picker model extended to audio
 * clips and (later) automation lanes.
 *
 * Stays narrow on purpose: one selection at a time, three possible
 * kinds. The playlist canvas reads `selection` on mousedown to decide
 * between rubber-band selection (default) and place-mode.
 */
import { create } from 'zustand'

export type PickerSelection =
  | { kind: 'pattern'; patternId: string }
  | { kind: 'audioClip'; trackId: string; clipId: string }
  | { kind: 'automation'; trackId: string; laneId: string }
  | null

interface PickerState {
  selection: PickerSelection
  selectAudioClip: (trackId: string, clipId: string) => void
  selectPattern: (patternId: string) => void
  selectAutomation: (trackId: string, laneId: string) => void
  /** Toggle: same item again clears selection. */
  toggleAudioClip: (trackId: string, clipId: string) => void
  togglePattern: (patternId: string) => void
  toggleAutomation: (trackId: string, laneId: string) => void
  clear: () => void
}

export const usePickerStore = create<PickerState>((set, get) => ({
  selection: null,
  selectAudioClip: (trackId, clipId) =>
    set({ selection: { kind: 'audioClip', trackId, clipId } }),
  selectPattern: (patternId) => set({ selection: { kind: 'pattern', patternId } }),
  selectAutomation: (trackId, laneId) =>
    set({ selection: { kind: 'automation', trackId, laneId } }),
  toggleAudioClip: (trackId, clipId) => {
    const cur = get().selection
    if (cur && cur.kind === 'audioClip' && cur.trackId === trackId && cur.clipId === clipId) {
      set({ selection: null })
    } else {
      set({ selection: { kind: 'audioClip', trackId, clipId } })
    }
  },
  togglePattern: (patternId) => {
    const cur = get().selection
    if (cur && cur.kind === 'pattern' && cur.patternId === patternId) {
      set({ selection: null })
    } else {
      set({ selection: { kind: 'pattern', patternId } })
    }
  },
  toggleAutomation: (trackId, laneId) => {
    const cur = get().selection
    if (cur && cur.kind === 'automation' && cur.trackId === trackId && cur.laneId === laneId) {
      set({ selection: null })
    } else {
      set({ selection: { kind: 'automation', trackId, laneId } })
    }
  },
  clear: () => set({ selection: null }),
}))
