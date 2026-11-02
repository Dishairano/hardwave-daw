import { beforeEach, describe, expect, it, vi } from 'vitest'

const calls: Array<[string, unknown]> = []
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args?: unknown) => {
    calls.push([cmd, args])
    if (cmd === 'get_keyboard_target') return { armed: [], keyboard: 'Lead' }
    return null
  }),
}))

const { arrivalAction, midiInputAppeared, resetMidiHotplugForTests, turnOff } = await import('./midiHotplug')
const { useSetupWizardStore } = await import('../stores/setupWizardStore')
const { useNotificationStore } = await import('../stores/notificationStore')
const { useLanguageStore } = await import('../i18n')

const akai = { port: 'Akai MPK Mini mk3', kind: 'keyboard' as const }
const desk = { port: 'Behringer X-Touch', kind: 'desk' as const }
const loop = { port: 'loopMIDI Port', kind: 'virtual' as const }

const sent = (cmd: string) => calls.filter(([c]) => c === cmd).map(([, args]) => args)
const toasts = () => useNotificationStore.getState().notifications

describe('what happens to an input that appears', () => {
  it('opens a controller never seen, and nothing it has seen before', () => {
    expect(arrivalAction(akai, {}, true)).toBe('open')
    expect(arrivalAction(akai, { [akai.port]: true }, true)).toBe('ignore')
    expect(arrivalAction(akai, { [akai.port]: false }, true)).toBe('ignore')
  })

  it('offers a desk as a control surface instead of opening it as a keyboard', () => {
    expect(arrivalAction(desk, {}, true)).toBe('offer-desk')
    expect(arrivalAction(desk, { [desk.port]: false }, true)).toBe('ignore')
  })

  it('leaves a virtual port alone', () => {
    expect(arrivalAction(loop, {}, true)).toBe('ignore')
  })

  it('opens nothing with MIDI off as a whole, and says it was found', () => {
    expect(arrivalAction(akai, {}, false)).toBe('found-midi-off')
  })
})

describe('a controller plugged in later', () => {
  beforeEach(() => {
    calls.length = 0
    resetMidiHotplugForTests()
    useLanguageStore.setState({ language: 'en' })
    useNotificationStore.getState().clear()
    useSetupWizardStore.setState({ visible: false, midiInputs: {}, midiMasterEnabled: true })
  })

  it('is opened, named, and says what its keys play, with Turn off', async () => {
    await midiInputAppeared(akai)
    expect(sent('open_midi_input')).toEqual([{ portName: akai.port }])
    const [toast] = toasts()
    expect(toast.message).toBe('Akai MPK Mini mk3 plugged in and turned on')
    expect(toast.detail).toBe('Its keys play the selected channel (Lead).')
    expect(toast.actions?.map(a => a.label)).toEqual(['Turn off', 'MIDI settings'])
  })

  it('stays off once turned off, for that device alone', async () => {
    await midiInputAppeared(akai)
    toasts()[0].actions![0].onClick()
    await vi.waitFor(() => expect(sent('close_midi_input')).toEqual([{ portName: akai.port }]))
    expect(useSetupWizardStore.getState().midiInputs[akai.port]).toBe(false)

    // Unplugged and plugged in again, or the next launch: it stays off.
    resetMidiHotplugForTests()
    calls.length = 0
    await midiInputAppeared(akai)
    expect(sent('open_midi_input')).toEqual([])

    // Another controller still opens by itself.
    await midiInputAppeared({ port: 'Novation Launchkey 49', kind: 'keyboard' })
    expect(sent('open_midi_input')).toEqual([{ portName: 'Novation Launchkey 49' }])
  })

  it('is not dealt with twice when the launch scan and the engine both report it', async () => {
    await midiInputAppeared(akai)
    await midiInputAppeared(akai)
    expect(sent('open_midi_input')).toHaveLength(1)
    expect(toasts()).toHaveLength(1)
  })

  it('a desk is offered, not opened, and Use as control surface makes it the desk', async () => {
    await midiInputAppeared(desk)
    expect(sent('open_midi_input')).toEqual([])
    const [toast] = toasts()
    expect(toast.message).toBe('Behringer X-Touch plugged in: a control surface')
    expect(toast.actions?.map(a => a.label)).toEqual(['Use as control surface', 'MIDI settings'])
    // Offered once.
    expect(useSetupWizardStore.getState().midiInputs[desk.port]).toBe(false)

    toast.actions![0].onClick()
    await vi.waitFor(() => expect(sent('set_control_surface_enabled')).toEqual([{ enabled: true }]))
    expect(sent('open_midi_input')).toEqual([{ portName: desk.port }])
    expect(sent('set_control_surface_input')).toEqual([{ input: desk.port }])
    expect(useSetupWizardStore.getState().midiInputs[desk.port]).toBe(true)
  })

  it('a virtual port is neither opened nor announced', async () => {
    await midiInputAppeared(loop)
    expect(calls).toEqual([])
    expect(toasts()).toEqual([])
  })

  it('with MIDI off, says it was found, and Turn on turns MIDI and it on', async () => {
    useSetupWizardStore.setState({ midiMasterEnabled: false })
    await midiInputAppeared(akai)
    expect(sent('open_midi_input')).toEqual([])
    const [toast] = toasts()
    expect(toast.message).toBe('Akai MPK Mini mk3 found')
    expect(toast.actions?.map(a => a.label)).toEqual(['Turn on'])
    toast.actions![0].onClick()
    await vi.waitFor(() => expect(sent('open_midi_input')).toEqual([{ portName: akai.port }]))
    expect(sent('set_midi_master_enabled')).toEqual([{ enabled: true }])
  })

  it('waits while the first run is open, and is dealt with when it closes', async () => {
    const { startMidiHotplug } = await import('./midiHotplug')
    const stop = await startMidiHotplug()
    useSetupWizardStore.setState({ visible: true })
    calls.length = 0
    await midiInputAppeared(akai)
    await midiInputAppeared(desk)
    expect(calls).toEqual([])
    expect(toasts()).toEqual([])

    // The first run's Controller step turned the Akai on itself.
    useSetupWizardStore.getState().rememberMidiInput(akai.port, true)
    useSetupWizardStore.setState({ visible: false })
    await vi.waitFor(() => expect(toasts()).toHaveLength(1))
    expect(toasts()[0].message).toBe('Behringer X-Touch plugged in: a control surface')
    stop()
  })

  it('Turn off remembers the device', async () => {
    await turnOff(akai.port)
    expect(useSetupWizardStore.getState().midiInputs[akai.port]).toBe(false)
  })
})
