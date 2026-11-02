import { describe, it, expect } from 'vitest'
import { useLanguageStore } from '../../i18n'
import { deskInputOptions, SURFACE_CAPTIONS } from './controlSurface'

describe('desk input picker', () => {
  it('puts the desk found by its name first, then the other inputs, then every input', () => {
    useLanguageStore.setState({ language: 'en' })
    expect(deskInputOptions(['Novation Launchkey 49', 'X-Touch'], 'X-Touch', true)).toEqual([
      { value: 'X-Touch', label: 'Desk: X-Touch (found)' },
      { value: 'Novation Launchkey 49', label: 'Desk: Novation Launchkey 49' },
      { value: '', label: 'Desk: every MIDI input' },
    ])
  })

  it('says found only for an input that was not chosen by hand', () => {
    useLanguageStore.setState({ language: 'en' })
    const labels = deskInputOptions(['X-Touch'], 'X-Touch', false).map(o => o.label)
    expect(labels).toEqual(['Desk: X-Touch', 'Desk: every MIDI input'])
  })

  it('keeps a chosen desk that is not plugged in, and offers every input with none', () => {
    useLanguageStore.setState({ language: 'en' })
    expect(deskInputOptions(['Akai MPK Mini mk3'], 'iCON P1-M', false).map(o => o.value))
      .toEqual(['iCON P1-M', 'Akai MPK Mini mk3', ''])
    expect(deskInputOptions([], null, false)).toEqual([{ value: '', label: 'Desk: every MIDI input' }])
  })

  it('reads in Dutch too', () => {
    useLanguageStore.setState({ language: 'nl' })
    expect(deskInputOptions(['X-Touch'], 'X-Touch', true).map(o => o.label))
      .toEqual(['Paneel: X-Touch (gevonden)', 'Paneel: elke MIDI-ingang'])
    useLanguageStore.setState({ language: 'en' })
  })

  it('has a caption for each protocol', () => {
    expect(SURFACE_CAPTIONS.mackie).toMatch(/^Mackie Control \(MCU\) and the desks that copy it/)
    expect(SURFACE_CAPTIONS.hui).toMatch(/four letters of each name/)
  })
})
