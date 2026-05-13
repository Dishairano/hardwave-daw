import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/**
 * Audio-engine behavioural preferences. UI-only at the time of
 * writing — the toggles persist the user's intent so the matching
 * backend wiring (Rust audio thread changes) can pick them up in a
 * follow-up ship without forcing the user to re-configure.
 *
 * Mirrors FL Studio's Audio Settings → Mixer settings options:
 *
 *  - resetPluginsOnTransport: when on, every plug-in's `reset()` runs
 *    on transport Stop / position jump so internal state (delay tails,
 *    LFO phase, oscillator phase) starts fresh.
 *  - playTruncatedNotes: when on, MidiTrackNode plays notes that were
 *    cut short by a playhead jump, fading in mid-note. When off,
 *    notes only fire from their start sample.
 */
interface AudioPrefsState {
  resetPluginsOnTransport: boolean
  playTruncatedNotes: boolean
  setResetPluginsOnTransport: (v: boolean) => void
  setPlayTruncatedNotes: (v: boolean) => void
}

export const useAudioPrefsStore = create<AudioPrefsState>()(
  persist(
    (set) => ({
      resetPluginsOnTransport: true,
      playTruncatedNotes: false,
      setResetPluginsOnTransport: (resetPluginsOnTransport) => set({ resetPluginsOnTransport }),
      setPlayTruncatedNotes: (playTruncatedNotes) => set({ playTruncatedNotes }),
    }),
    { name: 'hw-audio-prefs' },
  ),
)
