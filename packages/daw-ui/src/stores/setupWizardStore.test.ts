import { beforeEach, describe, expect, it, vi } from 'vitest'

const calls: Array<[string, unknown]> = []
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args?: unknown) => { calls.push([cmd, args]) }),
}))

const { applySavedMidiInputSettings, midiInputOn, useSetupWizardStore } = await import('./setupWizardStore')

describe('MIDI inputs chosen in the setup wizard', () => {
  beforeEach(() => {
    calls.length = 0
    useSetupWizardStore.setState({ midiInputs: {}, velocityCurves: {}, midiMasterEnabled: true })
  })

  it('are on until switched off', () => {
    expect(midiInputOn({}, 'Akai MPK Mini')).toBe(true)
    expect(midiInputOn({ 'Akai MPK Mini': false }, 'Akai MPK Mini')).toBe(false)
  })

  it('are opened when switched on and closed when switched off', async () => {
    await useSetupWizardStore.getState().setMidiInputEnabled('Akai MPK Mini', true)
    await useSetupWizardStore.getState().setMidiInputEnabled('loopMIDI Port', false)
    expect(calls).toEqual([
      ['open_midi_input', { portName: 'Akai MPK Mini' }],
      ['close_midi_input', { portName: 'loopMIDI Port' }],
    ])
  })

  it('are opened again at the next launch, and only the ones left on', async () => {
    await useSetupWizardStore.getState().setMidiInputEnabled('Akai MPK Mini', true)
    await useSetupWizardStore.getState().setMidiInputEnabled('loopMIDI Port', false)
    calls.length = 0
    applySavedMidiInputSettings()
    const opened = calls.filter(([cmd]) => cmd === 'open_midi_input').map(([, args]) => args)
    expect(opened).toEqual([{ portName: 'Akai MPK Mini' }])
  })
})
