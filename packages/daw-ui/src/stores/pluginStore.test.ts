import { beforeEach, describe, expect, it, vi } from 'vitest'

const known = [
  { id: 'hardwave.native.sampler', name: 'Sampler', vendor: 'Hardwave', version: '1', format: 'Native', category: 'Instrument', num_inputs: 0, num_outputs: 2, has_midi_input: true, has_editor: true },
  { id: 'vst3:abc', name: 'Hard-Shape', vendor: 'Hardwave Studios', version: '1', format: 'VST3', category: 'Effect', num_inputs: 2, num_outputs: 2, has_midi_input: false, has_editor: true },
]
const calls: string[] = []
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) => {
    calls.push(cmd)
    if (cmd === 'get_plugins') return known
    if (cmd === 'get_last_scan_diff') return { added: [], removed: [] }
    return null
  }),
}))

const { usePluginStore } = await import('./pluginStore')
const { tabListsPlugins } = await import('../components/browser/Browser')

describe('the browser lists the plug-ins the app already knows', () => {
  beforeEach(() => {
    calls.length = 0
    usePluginStore.setState({ plugins: [], scanning: false })
  })

  it('without a scan, as soon as a tab that lists plug-ins shows', async () => {
    expect(tabListsPlugins('lists')).toBe(true)
    expect(tabListsPlugins('favorites')).toBe(true)
    expect(tabListsPlugins('search')).toBe(true)
    expect(tabListsPlugins('folders')).toBe(false)
    await usePluginStore.getState().showKnownPlugins()
    expect(usePluginStore.getState().plugins.map((p) => p.name)).toEqual(['Sampler', 'Hard-Shape'])
    expect(calls).toEqual(['get_plugins'])
  })

  it('and leaves the list to a scan that is running', async () => {
    usePluginStore.setState({ scanning: true })
    await usePluginStore.getState().showKnownPlugins()
    expect(calls).toEqual([])
    expect(usePluginStore.getState().plugins).toEqual([])
  })
})
