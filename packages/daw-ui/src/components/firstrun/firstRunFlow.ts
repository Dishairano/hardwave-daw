import { STARTERS, type StarterId } from '../TemplateDialog'
import { t, tf } from '../../i18n'
import type { WizardStep } from '../../stores/setupWizardStore'

/**
 * The first run, as one sequence in one window: the start screen, the
 * sound card with a test tone, the controller (only when one is found),
 * and a song to start from.
 *
 * It used to be three windows that stacked: a MIDI-first setup wizard
 * under the start screen and over the splash, which came up again after
 * New project, and then the New project dialog, all before anything had
 * made a sound. These are the rules the window follows, kept apart from
 * it so they can be tested.
 */

export type FlowStep = WizardStep

export const FLOW_STEPS: { id: FlowStep; label: string }[] = [
  { id: 'start', label: 'Start' },
  { id: 'sound', label: 'Sound' },
  { id: 'controller', label: 'Controller' },
  { id: 'song', label: 'Song' },
]

/** Where Continue goes. The Controller step is there only when a
 *  controller was found; without one, Sound goes straight to Song. */
export function nextStep(step: FlowStep, controllersFound: boolean): FlowStep | null {
  switch (step) {
    case 'start': return 'sound'
    case 'sound': return controllersFound ? 'controller' : 'song'
    case 'controller': return 'song'
    case 'song': return null
  }
}

/** Where Back goes: the way Continue came. */
export function previousStep(step: FlowStep, controllersFound: boolean): FlowStep | null {
  switch (step) {
    case 'start': return null
    case 'sound': return 'start'
    case 'controller': return 'sound'
    case 'song': return controllersFound ? 'controller' : 'sound'
  }
}

export type StepState = 'done' | 'active' | 'todo' | 'skipped'

/** How the stepper on top shows each step: the ones behind as done, this
 *  one as active, and Controller struck through when none was found. */
export function stepStates(step: FlowStep, controllersFound: boolean): Record<FlowStep, StepState> {
  const at = FLOW_STEPS.findIndex(s => s.id === step)
  const states = {} as Record<FlowStep, StepState>
  FLOW_STEPS.forEach((s, i) => {
    if (s.id === 'controller' && !controllersFound && step !== 'controller') states[s.id] = 'skipped'
    else if (i < at) states[s.id] = 'done'
    else if (i === at) states[s.id] = 'active'
    else states[s.id] = 'todo'
  })
  return states
}

/** "Step 2 of 4". The count stays four with the Controller step left out,
 *  as the stepper still shows it, struck through. */
export function stepCounter(step: FlowStep): string {
  return tf('Step {n} of {total}', { n: FLOW_STEPS.findIndex(s => s.id === step) + 1, total: FLOW_STEPS.length })
}

/** What the Song step opens: a starter song, the empty project, or a song
 *  from disk. */
export type SongChoice = `starter:${StarterId}` | 'empty' | 'open'

/** Picked when the Song step opens: the Hardstyle starter, so the first
 *  press on Play makes a sound. */
export const DEFAULT_SONG: SongChoice = 'starter:hardstyle'

/** The Song step's button names what it opens. */
export function finishLabel(choice: SongChoice): string {
  if (choice === 'empty') return t('Open the empty project')
  if (choice === 'open') return t('Open project…')
  const starter = STARTERS.find(s => `starter:${s.id}` === choice)
  return tf('Open the {name} starter', { name: t(starter?.title ?? 'Hardstyle') })
}

/** The template a choice starts from. The empty project is a new song,
 *  which opens with Kick, Clap, Hat and Snare loaded, as its card says;
 *  Blank in File > New is the one with nothing in it. */
export function templateForChoice(choice: Exclude<SongChoice, 'open'>): 'drums' | `starter:${StarterId}` {
  return choice === 'empty' ? 'drums' : choice
}

/** A note number as the piano roll writes it: 60 is C4. */
export function noteName(note: number): string {
  const names = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
  return `${names[note % 12]}${Math.floor(note / 12) - 1}`
}
