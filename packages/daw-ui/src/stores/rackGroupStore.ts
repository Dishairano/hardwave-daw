import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

/** The song's channel rack groups by name, in the song's order. */
interface RackGroupState {
  groups: string[]
  /** Read them from the song; asked whenever the song's tracks change. */
  fetchGroups: () => Promise<void>
}

export const useRackGroupStore = create<RackGroupState>((set) => ({
  groups: [],
  fetchGroups: async () => {
    try {
      const groups = await invoke<string[]>('get_rack_groups')
      set({ groups: Array.isArray(groups) ? groups : [] })
    } catch {
      // A backend without groups: the filter shows All only.
      set({ groups: [] })
    }
  },
}))
