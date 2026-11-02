import { invoke } from '@tauri-apps/api/core'
import { useSetupWizardStore } from '../stores/setupWizardStore'
import { useNotificationStore } from '../stores/notificationStore'
import { t, tf } from '../i18n'

/**
 * Controllers plugged in later.
 *
 * The setup wizard turned on the controllers it found; one plugged in after
 * that showed up in Settings > MIDI with a Connect button and stayed off,
 * so its keys played nothing until the producer found that page. Now a
 * controller that has never been seen is opened the moment it appears, and
 * a message names it, with Turn off. Turn off is remembered for that device,
 * so it never opens by itself again. A control surface is offered as one
 * instead of being opened as a keyboard (its buttons would play notes), and
 * a virtual port is left alone (it could feed the DAW's own MIDI back in).
 *
 * The engine notices an input appear and says what kind it is by its name;
 * what to do with it is decided here, against the per-device memory the
 * first run and Settings > MIDI keep (`midiInputs`).
 */

/** What the engine says an input is, by its name. */
export type InputKind = 'keyboard' | 'desk' | 'virtual'

/** One MIDI input, as `list_midi_input_kinds` and `midi-input-appeared` give it. */
export interface MidiInputSeen {
  port: string
  kind: InputKind
}

export type ArrivalAction =
  /** Seen before (its choice stands), or a virtual port. */
  | 'ignore'
  /** Never seen: open it, and say so with Turn off. */
  | 'open'
  /** Never seen and named like a desk: offer it as a control surface. */
  | 'offer-desk'
  /** Never seen, but MIDI is off as a whole: say it was found, with Turn on. */
  | 'found-midi-off'

/**
 * What to do with an input that appeared.
 *
 * `seen` is the per-device memory: an input in it was turned on or off
 * before, by the first run, Settings > MIDI or a message like this one.
 * One that was on is opened again by the engine; one turned off stays off.
 */
export function arrivalAction(
  input: MidiInputSeen,
  seen: Record<string, boolean>,
  midiOn: boolean,
): ArrivalAction {
  if (Object.prototype.hasOwnProperty.call(seen, input.port)) return 'ignore'
  if (input.kind === 'virtual') return 'ignore'
  if (input.kind === 'desk') return 'offer-desk'
  return midiOn ? 'open' : 'found-midi-off'
}

/** Inputs dealt with this session, so the engine's notice and the scan at
 *  launch do not both act on one. */
const handled = new Set<string>()
/** Inputs that appeared while the first run was open: its Controller step
 *  deals with controllers itself, and what it leaves is dealt with when it
 *  closes. */
let waiting: MidiInputSeen[] = []

/** Open Settings on the MIDI tab. */
function openMidiSettings() {
  window.dispatchEvent(new CustomEvent('daw:openSettings', { detail: 'midi' }))
}

/** What a keyboard's keys play now, in words. */
async function whatItPlays(): Promise<string> {
  try {
    const target = await invoke<{ armed: string[]; keyboard: string | null }>('get_keyboard_target')
    if (target.armed.length > 0) {
      return tf('Its keys play the armed track ({names}).', { names: target.armed.join(', ') })
    }
    if (target.keyboard) return tf('Its keys play the selected channel ({name}).', { name: target.keyboard })
    return t('Its keys play nothing yet: add an instrument to hear them.')
  } catch {
    return t('Its keys play the selected channel.')
  }
}

/** Deal with an input that appeared, or that was there at launch. */
export async function midiInputAppeared(input: MidiInputSeen): Promise<void> {
  const store = useSetupWizardStore.getState()
  if (store.visible) {
    if (!waiting.some(w => w.port === input.port)) waiting.push(input)
    return
  }
  if (handled.has(input.port)) return
  handled.add(input.port)
  const { push } = useNotificationStore.getState()
  const name = input.port
  switch (arrivalAction(input, store.midiInputs, store.midiMasterEnabled)) {
    case 'ignore':
      return
    case 'open': {
      try {
        await store.setMidiInputEnabled(name, true)
      } catch (err) {
        push('warning', tf('{name} was found but would not open', { name }), {
          detail: String(err),
          actions: [{ label: t('MIDI settings'), onClick: openMidiSettings }],
        })
        return
      }
      push('info', tf('{name} plugged in and turned on', { name }), {
        detail: await whatItPlays(),
        actions: [
          { label: t('Turn off'), onClick: () => { void turnOff(name) } },
          { label: t('MIDI settings'), onClick: openMidiSettings },
        ],
      })
      return
    }
    case 'offer-desk':
      // Offered once: it stays off unless it is taken up, here or in
      // Settings > MIDI.
      store.rememberMidiInput(name, false)
      push('info', tf('{name} plugged in: a control surface', { name }), {
        detail: t('Not opened as a keyboard, so its buttons do not play notes.'),
        sticky: true,
        actions: [
          { label: t('Use as control surface'), onClick: () => { void takeAsControlSurface(name) } },
          { label: t('MIDI settings'), onClick: openMidiSettings },
        ],
      })
      return
    case 'found-midi-off':
      push('info', tf('{name} found', { name }), {
        detail: t('MIDI input is off in Settings > MIDI, so it was not turned on.'),
        actions: [{ label: t('Turn on'), onClick: () => { void turnOn(name) } }],
      })
  }
}

/** Turn off: close it, and remember that for this device. */
export async function turnOff(port: string): Promise<void> {
  try {
    await useSetupWizardStore.getState().setMidiInputEnabled(port, false)
  } catch { /* closing cannot fail in a way worth a word */ }
}

/** Turn on, from the message for MIDI switched off as a whole: MIDI on, and
 *  this controller with it. */
export async function turnOn(port: string): Promise<void> {
  const store = useSetupWizardStore.getState()
  store.setMidiMasterEnabled(true)
  try {
    await store.setMidiInputEnabled(port, true)
  } catch (err) {
    useNotificationStore.getState().push('warning', tf('{name} would not open', { name: port }), { detail: String(err) })
  }
}

/** Use as control surface: open the desk's input, read it as the desk, and
 *  switch the control surface on. */
export async function takeAsControlSurface(port: string): Promise<void> {
  const store = useSetupWizardStore.getState()
  try {
    await store.setMidiInputEnabled(port, true)
    await invoke('set_control_surface_input', { input: port })
    await invoke('set_control_surface_enabled', { enabled: true })
    useNotificationStore.getState().push('info', tf('{name} is the control surface', { name: port }), {
      detail: t('Its faders and buttons drive the mixer and the transport. The protocol is in Settings > MIDI.'),
      actions: [{ label: t('MIDI settings'), onClick: openMidiSettings }],
    })
  } catch (err) {
    useNotificationStore.getState().push('warning', tf('{name} would not open', { name: port }), { detail: String(err) })
  }
}

/** Deal with what appeared while the first run was open. */
function dealWithWaiting() {
  const queued = waiting
  waiting = []
  for (const input of queued) void midiInputAppeared(input)
}

/**
 * Start watching: the engine's notice for each input that appears, and one
 * look at the inputs there now, for a controller plugged in while the DAW
 * was closed. Called once at launch.
 */
export async function startMidiHotplug(): Promise<() => void> {
  const stops: Array<() => void> = []
  stops.push(useSetupWizardStore.subscribe((now, before) => {
    if (before.visible && !now.visible) dealWithWaiting()
  }))
  try {
    const { listen } = await import('@tauri-apps/api/event')
    stops.push(await listen<MidiInputSeen>('midi-input-appeared', ev => { void midiInputAppeared(ev.payload) }))
  } catch { /* not in Tauri */ }
  try {
    const present = await invoke<MidiInputSeen[]>('list_midi_input_kinds')
    for (const input of present) await midiInputAppeared(input)
  } catch { /* older backend: nothing to do */ }
  return () => { for (const stop of stops) stop() }
}

/** For tests: forget what this session dealt with. */
export function resetMidiHotplugForTests() {
  handled.clear()
  waiting = []
}
