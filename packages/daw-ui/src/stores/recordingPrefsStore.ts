import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/**
 * FL-style recording-related toolbar toggles.
 *
 * Each flag corresponds to a FL Studio toolbar widget surfaced by
 * the Ship 3a port of `toolbar-fl-parity-mockup`. The flags persist
 * through `zustand/middleware persist` so the user's recording
 * setup survives a relaunch, just like FL's per-project flags.
 *
 * Current state — FRONT-END UI ONLY: every flag is stored and the
 * matching toolbar button reflects + flips it, but the engine
 * doesn't yet observe these values. Wiring lives in follow-up ships:
 *
 *  - stepEditing      : when ON, Piano-Roll note entry advances the
 *                       playhead by Snap and arms the next step (FL
 *                       Ctrl+E). Needs Piano-Roll insertion mode.
 *  - waitForInput     : when ON, pressing Play parks the transport
 *                       in a "WAIT" state until the first MIDI event
 *                       (or a recording filter-matching event) is
 *                       received, then begins playback (FL Ctrl+I).
 *                       Needs a one-shot MIDI hook in transportStore.
 *  - blendRecord      : when ON, recording overdubs notes onto the
 *                       existing clip instead of overwriting; for
 *                       audio it implements sound-on-sound layering
 *                       (FL Ctrl+B). Needs MidiRecorder merge path
 *                       and an InputNode mix tap.
 *  - multilinkActive  : when ON, the next N tweaked controls record
 *                       linkage sequence for batch hardware mapping
 *                       (FL Ctrl+J). Needs Multilink record buffer.
 */
export interface RecordingPrefsState {
  stepEditing: boolean
  waitForInput: boolean
  blendRecord: boolean
  multilinkActive: boolean
  toggleStepEditing: () => void
  toggleWaitForInput: () => void
  toggleBlendRecord: () => void
  toggleMultilink: () => void
}

export const useRecordingPrefsStore = create<RecordingPrefsState>()(
  persist(
    (set) => ({
      stepEditing: false,
      waitForInput: false,
      blendRecord: false,
      multilinkActive: false,
      toggleStepEditing:   () => set((s) => ({ stepEditing: !s.stepEditing })),
      toggleWaitForInput:  () => set((s) => ({ waitForInput: !s.waitForInput })),
      toggleBlendRecord:   () => set((s) => ({ blendRecord: !s.blendRecord })),
      toggleMultilink:     () => set((s) => ({ multilinkActive: !s.multilinkActive })),
    }),
    { name: 'hw-recording-prefs' },
  ),
)
