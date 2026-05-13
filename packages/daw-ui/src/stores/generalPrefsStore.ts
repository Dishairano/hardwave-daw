import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/**
 * Global UI preferences from FL Studio's System Settings → General
 * page that map cleanly onto Hardwave today. Larger / FL-specific
 * options (auto-name channels, threaded project loading, .flp file
 * association, etc.) are tracked in the FL docs ledger as deferred.
 *
 * Each preference here drives a DOM-level CSS class or a global
 * utility so any component can opt in without each component carrying
 * its own subscription.
 */

export type NoteNamingConvention = 'english' | 'germanic' | 'solfege'

interface GeneralPrefsState {
  /** Pitch label scheme. English = C-B (default), Germanic = C-H (B
   * becomes H, A♯ becomes B), Solfège = Do-Ti (Do/Re/Mi/Fa/Sol/La/Ti). */
  noteNaming: NoteNamingConvention
  /** When false, the entire UI runs without transitions / animations.
   * Applies `.hw-no-animations` on `<html>`; the global stylesheet
   * short-circuits `transition` + `animation` for everything under it. */
  animationsEnabled: boolean
  /** Accessibility — bumps contrast on text and panel borders, adds a
   * focus ring on every interactive element. Applies `.hw-high-vis`
   * on `<html>`. */
  highVisibility: boolean

  setNoteNaming: (n: NoteNamingConvention) => void
  setAnimationsEnabled: (v: boolean) => void
  setHighVisibility: (v: boolean) => void
}

export const useGeneralPrefsStore = create<GeneralPrefsState>()(
  persist(
    (set) => ({
      noteNaming: 'english',
      animationsEnabled: true,
      highVisibility: false,
      setNoteNaming: (noteNaming) => set({ noteNaming }),
      setAnimationsEnabled: (animationsEnabled) => set({ animationsEnabled }),
      setHighVisibility: (highVisibility) => set({ highVisibility }),
    }),
    { name: 'hw-general-prefs' },
  ),
)

// ── Utilities ────────────────────────────────────────────────────────────

const ENGLISH = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
const GERMANIC = ['C', 'Cis', 'D', 'Dis', 'E', 'F', 'Fis', 'G', 'Gis', 'A', 'B', 'H']
const SOLFEGE = ['Do', 'Do#', 'Re', 'Re#', 'Mi', 'Fa', 'Fa#', 'Sol', 'Sol#', 'La', 'La#', 'Ti']

/** Convert a MIDI pitch number to a label using the active convention.
 * Stateless; reads the store synchronously so a re-rendered label
 * picks up changes on the next subscriber tick. Use the
 * `usePitchToName` hook for components that need reactivity. */
export function pitchToName(pitch: number, convention?: NoteNamingConvention): string {
  const conv = convention ?? useGeneralPrefsStore.getState().noteNaming
  const names = conv === 'germanic' ? GERMANIC : conv === 'solfege' ? SOLFEGE : ENGLISH
  const name = names[((pitch % 12) + 12) % 12]
  const octave = Math.floor(pitch / 12) - 1 // MIDI 60 = C4
  return `${name}${octave}`
}

/** React hook variant — subscribes to the store so the rendered label
 * updates when the user changes convention. */
export function usePitchToName(): (pitch: number) => string {
  const conv = useGeneralPrefsStore((s) => s.noteNaming)
  return (pitch) => pitchToName(pitch, conv)
}
