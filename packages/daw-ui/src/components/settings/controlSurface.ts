import { t, tf } from '../../i18n'

/** The languages a desk speaks, as the backend names them. */
export type DeskProtocol = 'mackie' | 'hui'

/** What `get_control_surface` says. */
export interface ControlSurfaceStatus {
  enabled: boolean
  /** Which bank of eight the strips are on, counting from zero. */
  bank: number
  bankCount: number
  protocol: DeskProtocol
  /** The MIDI input read as the desk; null reads every input. */
  input: string | null
  /** That input was found by its name rather than chosen. */
  found: boolean
}

export const SURFACE_OFF: ControlSurfaceStatus = {
  enabled: false, bank: 0, bankCount: 1, protocol: 'mackie', input: null, found: false,
}

export const PROTOCOL_OPTIONS: { value: DeskProtocol; label: string }[] = [
  { value: 'mackie', label: 'Mackie Control' },
  { value: 'hui', label: 'HUI' },
]

/** The line under the control surface row, for the protocol picked. */
export const SURFACE_CAPTIONS: Record<DeskProtocol, string> = {
  mackie: 'Mackie Control (MCU) and the desks that copy it: eight faders with mute, solo and arm, the V-pots for pan, the transport keys, and bank left and right. The motor faders, lights, LED rings and the scribble strips (name and level) follow the mix.',
  hui: 'HUI: eight faders with mute, solo and arm, the V-pots for pan, the transport keys, and bank left and right. The motor faders, lights and LED rings follow the mix; the scribble strips show four letters of each name. The desk is pinged once a second so it stays on line.',
}

/**
 * The desk input picker: the input read as the desk first (kept when it is
 * not open, so a chosen desk that is unplugged still shows), the other
 * open inputs, then every input, whose value is ''.
 */
export function deskInputOptions(
  openInputs: string[],
  input: string | null,
  found: boolean,
): { value: string; label: string }[] {
  const ports = input ? [input, ...openInputs.filter(p => p !== input)] : openInputs
  return [
    ...ports.map(port => ({
      value: port,
      label: found && port === input
        ? tf('Desk: {input} (found)', { input: port })
        : tf('Desk: {input}', { input: port }),
    })),
    { value: '', label: t('Desk: every MIDI input') },
  ]
}
