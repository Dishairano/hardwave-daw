import type { InputChannel } from '../../../stores/trackStore'

/** One entry of the Records menu for the audio interface. */
export interface InterfaceInputOption {
  value: string
  label: string
  channel: InputChannel
}

/** How an interface input is kept in the menu's value: `in:2:m`, `in:0:s`. */
export function inputValue(channel: InputChannel): string {
  return `in:${channel.first}:${channel.mono ? 'm' : 's'}`
}

export function parseInputValue(value: string): InputChannel | null {
  const m = /^in:(\d+):([ms])$/.exec(value)
  return m ? { first: Number(m[1]), mono: m[2] === 'm' } : null
}

/**
 * The interface's inputs as the Records menu lists them: the stereo pairs
 * first (In 1-2, In 3-4...), then each channel on its own, for a
 * microphone or a guitar on one input.
 */
export function interfaceInputs(count: number): InterfaceInputOption[] {
  const n = Math.max(0, Math.floor(count))
  const options: InterfaceInputOption[] = []
  for (let first = 0; first + 1 < n; first += 2) {
    const channel = { first, mono: false }
    options.push({ value: inputValue(channel), label: `In ${first + 1}-${first + 2}`, channel })
  }
  for (let first = 0; first < n; first++) {
    const channel = { first, mono: true }
    options.push({ value: inputValue(channel), label: `In ${first + 1}`, channel })
  }
  return options
}
