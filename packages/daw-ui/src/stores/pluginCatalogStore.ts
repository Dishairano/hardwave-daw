import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

/**
 * Lazy plug-in catalog for the Phase 3 picker.
 *
 * The legacy `pluginStore` triggers a `scan_plugins` IPC at app boot —
 * that's the right call for the Settings page, but the mixer picker
 * doesn't need it to fire on every project load. This store stays cold
 * until the user opens the flyout / modal, at which point a single
 * scan_plugins IPC populates `plugins` and every subsequent picker open
 * uses the cached list.
 *
 * Shape mirrors the Rust `PluginDescriptor` (crates/hardwave-plugin-host
 * /src/types.rs:27) — note `format` and `category` are stringly-typed
 * enums on the wire ("Vst3"|"Clap", "Effect"|"Instrument"|...).
 */

export type PluginFormat = 'Vst3' | 'Clap'
export type PluginCategory = 'Effect' | 'Instrument' | 'Analyzer' | 'Other'

export interface PluginDescriptor {
  id: string
  name: string
  vendor: string
  version: string
  format: PluginFormat
  /** Filesystem path. Native plug-ins serialize as the literal "<native>". */
  path: string
  category: PluginCategory
  num_inputs: number
  num_outputs: number
  has_midi_input: boolean
  has_editor: boolean
}

/** UI-facing category bucket. "Hardwave" is a vendor-derived pseudo-category. */
export type PickerCategory = 'All' | 'Hardwave' | 'Instrument' | 'Effect' | 'Analyzer' | 'Other'

/**
 * A plug-in is "native" (red H icon) when it's shipped by Hardwave. We
 * key on `vendor === 'Hardwave'` rather than the serialized path so a
 * future Hardwave VST3 build of LoudLab still gets the badge — see
 * crates/hardwave-native-plugins/src/eq.rs:69 where every native sets
 * `vendor: "Hardwave"`.
 */
export function isHardwaveNative(p: PluginDescriptor): boolean {
  return p.vendor === 'Hardwave'
}

interface PluginCatalogState {
  plugins: PluginDescriptor[]
  loaded: boolean
  loading: boolean
  /** Last error from a rescan; surfaced in the modal footer if non-null. */
  error: string | null

  /** Force a fresh scan + replace the cache. */
  rescan: () => Promise<void>
  /** Idempotent: triggers the first scan, no-op thereafter. */
  ensureLoaded: () => Promise<void>
  byId: (id: string) => PluginDescriptor | undefined
  byCategory: (cat: PickerCategory) => PluginDescriptor[]
  /** Free-text filter across name + vendor + category. Empty query
   *  returns every plug-in. */
  search: (q: string) => PluginDescriptor[]
}

export const usePluginCatalogStore = create<PluginCatalogState>((set, get) => ({
  plugins: [],
  loaded: false,
  loading: false,
  error: null,

  rescan: async () => {
    if (get().loading) return
    set({ loading: true, error: null })
    try {
      const plugins = await invoke<PluginDescriptor[]>('scan_plugins')
      set({ plugins, loaded: true, loading: false })
    } catch (e) {
      set({ loading: false, error: String(e) })
    }
  },

  ensureLoaded: async () => {
    const { loaded, loading } = get()
    if (loaded || loading) return
    await get().rescan()
  },

  byId: (id) => get().plugins.find((p) => p.id === id),

  byCategory: (cat) => {
    const all = get().plugins
    if (cat === 'All') return all
    if (cat === 'Hardwave') return all.filter(isHardwaveNative)
    return all.filter((p) => p.category === cat)
  },

  search: (q) => {
    const needle = q.trim().toLowerCase()
    const all = get().plugins
    if (!needle) return all
    return all.filter((p) =>
      (p.name + ' ' + p.vendor + ' ' + p.category).toLowerCase().includes(needle),
    )
  },
}))
