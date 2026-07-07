import { create } from 'zustand'

export interface MissingPluginInfo {
  pluginId: string
  trackId: string
  trackName: string
  slotId: string
  slotIndex: number
}

interface MissingPluginsState {
  /** Slots whose plugin isn't installed/scanned. Set on project load. */
  missing: MissingPluginInfo[]
  /** Banner hidden for the rest of the session (per project load). */
  dismissed: boolean
  set: (missing: MissingPluginInfo[]) => void
  dismiss: () => void
}

export const useMissingPluginsStore = create<MissingPluginsState>((set) => ({
  missing: [],
  dismissed: false,
  // A fresh load result un-dismisses: new project, new information.
  set: (missing) => set({ missing, dismissed: false }),
  dismiss: () => set({ dismissed: true }),
}))
