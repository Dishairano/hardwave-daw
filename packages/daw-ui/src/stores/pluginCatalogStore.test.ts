import { describe, expect, it } from 'vitest'
import { hasSuitePlugins, isHardwaveNative } from './pluginCatalogStore'

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

describe('the browser note that points at the Suite', () => {
  const builtIns = [
    { id: 'hardwave.native.eq', vendor: 'Hardwave' },
    { id: 'hardwave.native.sampler', vendor: 'Hardwave' },
  ]

  it('shows on a fresh install, where only the built-ins are there', () => {
    expect(hasSuitePlugins(builtIns)).toBe(false)
  })

  it('goes once a Hardwave plug-in from the Suite is found', () => {
    expect(hasSuitePlugins([...builtIns, { id: 'vst3:wettboi', vendor: 'Hardwave Studios' }])).toBe(true)
  })

  it('stays for other plug-ins', () => {
    expect(hasSuitePlugins([...builtIns, { id: 'vst3:serum', vendor: 'Xfer Records' }])).toBe(false)
  })
})
