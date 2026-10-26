import type { FollowMode } from '../components/arrangement/follow'
import type { RulerUnits, TimecodeFps } from '../components/arrangement/rulerLabel'
import { invoke } from '@tauri-apps/api/core'
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
  /** When true, the playlist and piano roll scroll along with the playhead
   * while the song plays. Off by default: the view jumping while you work
   * on a part further along is worse than losing sight of the playhead,
   * and the FOLLOW buttons turn it on for the moment you want it. */
  followPlayhead: boolean
  /** How the playlist follows: sliding under the playhead, or a page at a
   *  time (View > Follow). */
  followMode: FollowMode
  /** What the playlist ruler counts in: bars, minutes and seconds, or
   *  timecode. Bars for music, the other two for picture and sound design,
   *  which had no way to read the timeline at all. */
  rulerUnits: RulerUnits
  /** The frame rate timecode is counted in, on the ruler and in the MIDI
   *  timecode sent out; the ruler always counted 25. */
  timecodeFps: TimecodeFps
  /** The FPS meter overlay (Tools > Show FPS meter, Ctrl+Shift+F). */
  showFpsMeter: boolean
  /** The song template the DAW starts with, by name; none starts empty. */
  startupTemplate: string | null
  setStartupTemplate: (name: string | null) => void

  setNoteNaming: (n: NoteNamingConvention) => void
  setAnimationsEnabled: (v: boolean) => void
  setHighVisibility: (v: boolean) => void
  setFollowPlayhead: (v: boolean) => void
  setFollowMode: (v: FollowMode) => void
  setRulerUnits: (v: RulerUnits) => void
  setTimecodeFps: (v: TimecodeFps) => void
  toggleFpsMeter: () => void
}

export const useGeneralPrefsStore = create<GeneralPrefsState>()(
  persist(
    (set) => ({
      noteNaming: 'english',
      animationsEnabled: true,
      highVisibility: false,
      followPlayhead: false,
      followMode: 'continuous',
      rulerUnits: 'bars',
      timecodeFps: 25,
      showFpsMeter: false,
      startupTemplate: null,
      setStartupTemplate: (startupTemplate) => set({ startupTemplate }),
      setNoteNaming: (noteNaming) => set({ noteNaming }),
      setAnimationsEnabled: (animationsEnabled) => set({ animationsEnabled }),
      setHighVisibility: (highVisibility) => set({ highVisibility }),
      setFollowPlayhead: (followPlayhead) => set({ followPlayhead }),
      setFollowMode: (followMode) => set({ followMode }),
      setRulerUnits: (rulerUnits) => set({ rulerUnits }),
      setTimecodeFps: (timecodeFps) => {
        set({ timecodeFps })
        // The timecode sent to other gear runs at the same rate.
        invoke('set_midi_mtc_fps', { fps: timecodeFps }).catch(() => {})
      },
      toggleFpsMeter: () => set((s) => ({ showFpsMeter: !s.showFpsMeter })),
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
