import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

interface PluginDescriptor {
  id: string
  name: string
  vendor: string
  version: string
  format: string
  category: string
  num_inputs: number
  num_outputs: number
  has_midi_input: boolean
  has_editor: boolean
}

export interface ScanDiff {
  added: string[]
  removed: string[]
}

interface PluginState {
  plugins: PluginDescriptor[]
  scanning: boolean
  lastDiff: ScanDiff
  blocklist: string[]
  customVst3Paths: string[]
  customClapPaths: string[]
  cachePath: string | null

  scanPlugins: () => Promise<void>
  loadCachedPlugins: () => Promise<void>
  refreshScanDiff: () => Promise<void>
  loadBlocklist: () => Promise<void>
  setBlocklist: (ids: string[]) => Promise<void>
  toggleBlocked: (id: string) => Promise<void>
  loadCustomPaths: () => Promise<void>
  setCustomPaths: (vst3: string[], clap: string[]) => Promise<void>
  loadCachePath: () => Promise<void>
  addToTrack: (trackId: string, pluginId: string) => Promise<string>
  removeFromTrack: (trackId: string, slotId: string) => Promise<void>
}

export const usePluginStore = create<PluginState>((set, get) => ({
  plugins: [],
  scanning: false,
  lastDiff: { added: [], removed: [] },
  blocklist: [],
  customVst3Paths: [],
  customClapPaths: [],
  cachePath: null,

  scanPlugins: async () => {
    set({ scanning: true })
    const plugins = await invoke<PluginDescriptor[]>('scan_plugins')
    const lastDiff = await invoke<ScanDiff>('get_last_scan_diff')
    set({ plugins, lastDiff, scanning: false })
  },

  loadCachedPlugins: async () => {
    const plugins = await invoke<PluginDescriptor[]>('get_plugins')
    const lastDiff = await invoke<ScanDiff>('get_last_scan_diff')
    set({ plugins, lastDiff })
  },

  refreshScanDiff: async () => {
    const lastDiff = await invoke<ScanDiff>('get_last_scan_diff')
    set({ lastDiff })
  },

  loadBlocklist: async () => {
    const ids = await invoke<string[]>('get_plugin_blocklist')
    set({ blocklist: ids })
  },

  setBlocklist: async (ids) => {
    await invoke('set_plugin_blocklist', { ids })
    set({ blocklist: [...ids].sort() })
  },

  toggleBlocked: async (id) => {
    const current = new Set(get().blocklist)
    if (current.has(id)) current.delete(id)
    else current.add(id)
    const next = Array.from(current).sort()
    await invoke('set_plugin_blocklist', { ids: next })
    set({ blocklist: next })
  },

  loadCustomPaths: async () => {
    const [vst3, clap] = await invoke<[string[], string[]]>('get_custom_scan_paths')
    set({ customVst3Paths: vst3, customClapPaths: clap })
  },

  setCustomPaths: async (vst3, clap) => {
    await invoke('set_custom_scan_paths', { vst3, clap })
    set({ customVst3Paths: vst3, customClapPaths: clap })
  },

  loadCachePath: async () => {
    const path = await invoke<string | null>('plugin_cache_path')
    set({ cachePath: path })
  },

  addToTrack: async (trackId, pluginId) => {
    return await invoke<string>('add_plugin_to_track', { trackId, pluginId })
  },

  removeFromTrack: async (trackId, slotId) => {
    await invoke('remove_plugin_from_track', { trackId, slotId })
  },
}))
