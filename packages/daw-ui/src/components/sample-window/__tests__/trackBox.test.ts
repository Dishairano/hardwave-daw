import { describe, it, expect } from 'vitest'
import { trackBoxValue } from '../SampleWindow'
import type { TrackInfo } from '../../../stores/trackStore'

const t = (id: string, name: string, kind: string, mixerNumber: number | null, outputBus: string | null = null) =>
  ({ id, name, kind, mixerNumber, outputBus } as unknown as TrackInfo)

describe('the Track box', () => {
  const master = t('master', 'Master', 'Master', 0)
  // A song made before: every track has an insert of its own, numbered as
  // the mixer counted them, whatever its id says.
  const kick = t('insert-001', 'Kick', 'Midi', 1)
  const drums = t('insert-007', 'Drums', 'Audio', 9)
  // FL's shape: channels with no insert of their own, routed to one.
  const routed = t('4f1c-uuid', 'Hardwave Sampler', 'Midi', null, 'insert-007')
  const loose = t('9a2b-uuid', 'Hardwave Sampler', 'Midi', null, null)
  const auto = t('auto', 'Volume', 'Automation', null)
  const tracks = [master, kick, drums, routed, loose, auto]

  it('shows the insert a channel plays through by the mixer number, never by its id', () => {
    expect(trackBoxValue(kick, tracks)).toEqual({ value: '1', name: 'Kick' })
    expect(trackBoxValue(routed, tracks)).toEqual({ value: '9', name: 'Drums' })
    expect(trackBoxValue(loose, tracks)).toEqual({ value: 'M', name: 'Master' })
  })

  it('a channel with its own insert routed to another shows where it goes', () => {
    const lead = t('lead', 'Lead', 'Midi', 3, 'insert-007')
    expect(trackBoxValue(lead, [...tracks, lead])).toEqual({ value: '9', name: 'Drums' })
  })

  it('an automation channel has no insert', () => {
    expect(trackBoxValue(auto, tracks).value).toBe('–')
  })
})
