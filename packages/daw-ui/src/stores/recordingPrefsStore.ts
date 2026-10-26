import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { invoke } from '@tauri-apps/api/core'

/**
 * FL-style recording-related toolbar toggles.
 *
 * Each flag corresponds to a FL Studio toolbar widget surfaced by
 * the Ship 3a port of `toolbar-fl-parity-mockup`. The flags persist
 * through `zustand/middleware persist` so the user's recording
 * setup survives a relaunch, just like FL's per-project flags.
 *
 * All four are wired.
 *
 *  - stepEditing      : WIRED (2026-09-17, FL Ctrl+E) — with it ON the
 *                       typing keyboard writes each note into the open
 *                       clip at the edit cursor and advances the cursor
 *                       by Snap. With it OFF the keys only play, which
 *                       is what the button always claimed: the piano
 *                       roll used to write a note either way.
 *  - waitForInput     : WIRED (2026-07-08, FL Ctrl+I) — Play/Record
 *                       park the transport until the first MIDI
 *                       event arrives (engine `wait_pending`; synced
 *                       via `set_wait_for_input`, re-applied at boot
 *                       by `syncWaitForInput`).
 *  - blendRecord      : WIRED (2026-07-08, FL Ctrl+B) — recorded MIDI
 *                       merges into the overlapping clip instead of
 *                       stacking a new one (backend
 *                       merge_notes_into_overlapping_clip; audio
 *                       recording already layers takes).
 *  - multilinkActive  : WIRED (2026-09-30, FL Ctrl+J) — mapping a
 *                       controller meant opening the MIDI mappings
 *                       dialog once per knob. With it ON, "MIDI learn"
 *                       on a control arms the learn straight away and
 *                       stays armed for the next control, so a whole
 *                       controller is mapped in one pass. The number of
 *                       links made in the pass is kept here for the
 *                       toolbar to show.
 */
export interface RecordingPrefsState {
  stepEditing: boolean
  waitForInput: boolean
  blendRecord: boolean
  /** Recorded MIDI replaces the notes it is played over, instead of
   *  landing on top of them in a second clip that plays as well. Blend
   *  and replace exclude each other; with neither, a take is a new clip. */
  replaceRecord: boolean
  multilinkActive: boolean
  /** Recorded MIDI notes land on the playlist's snap grid. The backend
   *  took a grid all along and the interface always sent none. */
  quantizeRecording: boolean
  /** Links made since multilink was armed. Reset each time it is armed. */
  multilinkCount: number
  noteMultilinkLink: () => void
  toggleStepEditing: () => void
  toggleWaitForInput: () => void
  toggleBlendRecord: () => void
  setMidiTakeMode: (mode: 'stack' | 'blend' | 'replace') => void
  toggleMultilink: () => void
  toggleQuantizeRecording: () => void
}

export const useRecordingPrefsStore = create<RecordingPrefsState>()(
  persist(
    (set) => ({
      stepEditing: false,
      waitForInput: false,
      blendRecord: false,
      replaceRecord: false,
      multilinkActive: false,
      quantizeRecording: false,
      multilinkCount: 0,
      toggleStepEditing:   () => set((s) => ({ stepEditing: !s.stepEditing })),
      toggleWaitForInput:  () => set((s) => {
        const next = !s.waitForInput
        // Engine observes this via the command; failure is non-fatal
        // (browser/mock mode) — the persisted flag re-syncs at boot.
        invoke('set_wait_for_input', { enabled: next }).catch(() => {})
        return { waitForInput: next }
      }),
      toggleBlendRecord:   () => set((s) => ({ blendRecord: !s.blendRecord, replaceRecord: false })),
      setMidiTakeMode:     (mode) => set({ blendRecord: mode === 'blend', replaceRecord: mode === 'replace' }),
      toggleMultilink:     () => set((s) => {
        const next = !s.multilinkActive
        // Switching it off cancels a learn still waiting for a knob that
        // is never going to move.
        if (!next) invoke('midi_learn_cancel').catch(() => {})
        return { multilinkActive: next, multilinkCount: 0 }
      }),
      noteMultilinkLink:   () => set((s) => ({ multilinkCount: s.multilinkCount + 1 })),
      toggleQuantizeRecording: () => set((s) => ({ quantizeRecording: !s.quantizeRecording })),
    }),
    {
      name: 'hw-recording-prefs',
      // The count belongs to one mapping pass, not to the machine, so it
      // is not written to disk.
      partialize: (s) => ({
        stepEditing: s.stepEditing,
        waitForInput: s.waitForInput,
        blendRecord: s.blendRecord,
        replaceRecord: s.replaceRecord,
        multilinkActive: s.multilinkActive,
        quantizeRecording: s.quantizeRecording,
      }) as RecordingPrefsState,
    },
  ),
)

/** Re-apply the persisted wait-for-input flag to the engine at boot. */
export function syncWaitForInput() {
  const enabled = useRecordingPrefsStore.getState().waitForInput
  if (enabled) invoke('set_wait_for_input', { enabled }).catch(() => {})
}
