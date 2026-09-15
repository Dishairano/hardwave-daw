import { describe, it, expect, beforeEach } from 'vitest'
import {
  useBrowserStore,
  normalizeDiskPath,
  isSameOrInsideDiskPath,
} from '../browserStore'

beforeEach(() => useBrowserStore.setState({ diskRoots: [], expandedDiskPaths: new Set() }))

describe('normalizeDiskPath', () => {
  it('strips trailing separators but keeps filesystem roots whole', () => {
    expect(normalizeDiskPath('  /Users/me/Samples/ ')).toBe('/Users/me/Samples')
    expect(normalizeDiskPath('D:\\Packs\\Hardstyle\\')).toBe('D:\\Packs\\Hardstyle')
    expect(normalizeDiskPath('/')).toBe('/')
    expect(normalizeDiskPath('C:')).toBe('C:\\')
    expect(normalizeDiskPath('C:\\')).toBe('C:\\')
  })
})

describe('isSameOrInsideDiskPath', () => {
  it('matches the root and anything below it, not siblings with a shared prefix', () => {
    expect(isSameOrInsideDiskPath('/s/Kicks', '/s/Kicks')).toBe(true)
    expect(isSameOrInsideDiskPath('/s/Kicks/Hard', '/s/Kicks')).toBe(true)
    expect(isSameOrInsideDiskPath('/s/Kicks2', '/s/Kicks')).toBe(false)
    expect(isSameOrInsideDiskPath('D:\\Packs\\Loops', 'D:\\Packs')).toBe(true)
    expect(isSameOrInsideDiskPath('D:\\Packs2', 'D:\\Packs')).toBe(false)
    expect(isSameOrInsideDiskPath('C:\\Samples', 'C:\\')).toBe(true)
    expect(isSameOrInsideDiskPath('/anything', '/')).toBe(true)
  })
})

describe('browserStore disk roots', () => {
  const s = () => useBrowserStore.getState()

  it('adding a folder opens it and ignores a duplicate spelled with a trailing slash', () => {
    s().addDiskRoot('/Samples/')
    s().addDiskRoot('/Samples')
    expect(s().diskRoots).toEqual(['/Samples'])
    expect(s().expandedDiskPaths.has('/Samples')).toBe(true)
  })

  it('removing a root forgets the folders opened under it, but not under other roots', () => {
    s().addDiskRoot('/Samples')
    s().addDiskRoot('/Samples/Kicks')
    s().addDiskRoot('/Loops')
    s().toggleDiskPathExpanded('/Samples/Kicks/Hard')
    s().toggleDiskPathExpanded('/Loops/140')

    s().removeDiskRoot('/Loops')
    expect(s().diskRoots).toEqual(['/Samples', '/Samples/Kicks'])
    expect(s().expandedDiskPaths.has('/Loops')).toBe(false)
    expect(s().expandedDiskPaths.has('/Loops/140')).toBe(false)

    // /Samples still contains /Samples/Kicks/Hard, so removing the nested
    // root keeps that folder open.
    s().removeDiskRoot('/Samples/Kicks')
    expect(s().expandedDiskPaths.has('/Samples/Kicks/Hard')).toBe(true)
  })

  it('toggling a folder opens and closes it', () => {
    s().toggleDiskPathExpanded('/Samples/Snares')
    expect(s().expandedDiskPaths.has('/Samples/Snares')).toBe(true)
    s().toggleDiskPathExpanded('/Samples/Snares')
    expect(s().expandedDiskPaths.has('/Samples/Snares')).toBe(false)
  })
})
