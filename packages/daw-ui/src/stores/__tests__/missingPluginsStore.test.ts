import { describe, it, expect, beforeEach } from 'vitest'
import { useMissingPluginsStore } from '../missingPluginsStore'

const mk = (id: string) => ({
  pluginId: id, trackId: 't1', trackName: 'Kick', slotId: 's1', slotIndex: 0,
})

beforeEach(() => useMissingPluginsStore.setState({ missing: [], dismissed: false }))

describe('missingPluginsStore', () => {
  it('a new load result un-dismisses the banner', () => {
    const s = () => useMissingPluginsStore.getState()
    s().set([mk('vst3:gone')])
    s().dismiss()
    expect(s().dismissed).toBe(true)
    // Next project load brings fresh information — banner returns.
    s().set([mk('vst3:other')])
    expect(s().dismissed).toBe(false)
  })

  it('rescan resolving everything empties the list', () => {
    const s = () => useMissingPluginsStore.getState()
    s().set([mk('a'), mk('b')])
    s().set([])
    expect(s().missing).toHaveLength(0)
  })
})
