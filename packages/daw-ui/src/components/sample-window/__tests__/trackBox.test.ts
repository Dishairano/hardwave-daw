import { describe, it, expect } from 'vitest'
import { insertNumber, trackBoxValue } from '../SampleWindow'
import type { TrackInfo } from '../../../stores/trackStore'

const t = (id: string, name: string, kind = 'Midi', outputBus: string | null = null) =>
  ({ id, name, kind, outputBus } as unknown as TrackInfo)

describe('the Track box', () => {
  const master = t('master', 'Master', 'Master')
  const kick = t('insert-001', 'Kick')
  const drums = t('insert-007', 'Drums', 'Audio')
  const routed = t('4f1c-uuid', 'Hardwave Sampler', 'Midi', 'insert-007')
  const loose = t('9a2b-uuid', 'Hardwave Sampler', 'Midi', null)
  const tracks = [master, kick, drums, routed, loose]

  it('reads a mixer insert number from its id', () => {
    expect(insertNumber('insert-007')).toBe(7)
    expect(insertNumber('insert-501x')).toBeNull()
    expect(insertNumber('4f1c-uuid')).toBeNull()
  })

  it('shows the insert a channel plays through, never its place in the track list', () => {
    expect(trackBoxValue(kick, tracks)).toEqual({ value: '1', name: 'Master' })
    expect(trackBoxValue(routed, tracks)).toEqual({ value: '7', name: 'Drums' })
    expect(trackBoxValue(loose, tracks)).toEqual({ value: 'M', name: 'Master' })
  })
})
