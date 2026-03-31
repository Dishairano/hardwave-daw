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

interface PluginState {
  plugins: PluginDescriptor[]
  scanning: boolean
  scanPlugins: () => Promise<void>
  addToTrack: (trackId: string, pluginId: string) => Promise<string>
  removeFromTrack: (trackId: string, slotId: string) => Promise<void>
}

export const usePluginStore = create<PluginState>((set) => ({
  plugins: [],
  scanning: false,

  scanPlugins: async () => {
    set({ scanning: true })
    const plugins = await invoke<PluginDescriptor[]>('scan_plugins')
    set({ plugins, scanning: false })
  },

  addToTrack: async (trackId, pluginId) => {
    return await invoke<string>('add_plugin_to_track', { trackId, pluginId })
  },

  removeFromTrack: async (trackId, slotId) => {
    await invoke('remove_plugin_from_track', { trackId, slotId })
  },
}))
