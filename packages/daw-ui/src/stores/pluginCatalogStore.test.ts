import { describe, expect, it } from 'vitest'
import { isHardwaveNative } from './pluginCatalogStore'

describe('the Hardwave list in the plug-in picker', () => {
  it('holds the built-ins', () => {
    expect(isHardwaveNative({ vendor: 'Hardwave' })).toBe(true)
  })

  it('holds the plug-ins the Suite installs, which say Hardwave Studios', () => {
    expect(isHardwaveNative({ vendor: 'Hardwave Studios' })).toBe(true)
  })

  it('leaves everyone else out', () => {
    expect(isHardwaveNative({ vendor: 'Xfer Records' })).toBe(false)
    expect(isHardwaveNative({ vendor: '' })).toBe(false)
  })
})
