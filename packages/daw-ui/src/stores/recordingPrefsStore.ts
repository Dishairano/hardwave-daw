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
 * Three of the four are wired. `multilinkActive` is the exception and has
 * no button in the toolbar for that reason:
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
      toggleWaitForInput:  () => set((s) => {
        const next = !s.waitForInput
        // Engine observes this via the command; failure is non-fatal
        // (browser/mock mode) — the persisted flag re-syncs at boot.
        invoke('set_wait_for_input', { enabled: next }).catch(() => {})
        return { waitForInput: next }
      }),
      toggleBlendRecord:   () => set((s) => ({ blendRecord: !s.blendRecord })),
      toggleMultilink:     () => set((s) => ({ multilinkActive: !s.multilinkActive })),
    }),
    { name: 'hw-recording-prefs' },
  ),
)

/** Re-apply the persisted wait-for-input flag to the engine at boot. */
export function syncWaitForInput() {
  const enabled = useRecordingPrefsStore.getState().waitForInput
  if (enabled) invoke('set_wait_for_input', { enabled }).catch(() => {})
}
