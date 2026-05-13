import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/**
 * Autosave cadence preference. Mirrors FL Studio's "Backup (Autosave)"
 * options under File Settings — six steps from Never to Very Frequently
 * plus the "before risky operations" flag the busiest setting implies.
 *
 * The interval drives `App.tsx`'s autosave loop (the one that pings
 * `autosave_save` while the project is dirty). Disabled when frequency
 * is 'never' so the autosave timer never fires; the user still gets a
 * 10-minute reminder via the dirty-prompt path.
 */

export type AutosaveFrequency =
  | 'never'
  | 'rarely'
  | 'occasionally'
  | 'regularly'
  | 'frequently'
  | 'very-frequently'

/** Map a frequency option to its interval in milliseconds. The 'never'
 * label still returns a value because the App-level effect uses it
 * with a no-op check, but in practice the timer is short-circuited. */
export function frequencyIntervalMs(f: AutosaveFrequency): number {
  switch (f) {
    case 'never': return 0
    case 'rarely': return 15 * 60 * 1000
    case 'occasionally': return 10 * 60 * 1000
    case 'regularly': return 5 * 60 * 1000
    case 'frequently': return 5 * 60 * 1000 // + before risky ops
    case 'very-frequently': return 60 * 1000 // + before risky ops
  }
}

export const AUTOSAVE_OPTIONS: Array<{ id: AutosaveFrequency; label: string; desc: string }> = [
  { id: 'never', label: 'Never', desc: 'No autosave (reminder every 10 min while dirty)' },
  { id: 'rarely', label: 'Rarely', desc: 'Every 15 minutes' },
  { id: 'occasionally', label: 'Occasionally', desc: 'Every 10 minutes' },
  { id: 'regularly', label: 'Regularly', desc: 'Every 5 minutes' },
  { id: 'frequently', label: 'Frequently', desc: 'Every 5 minutes + before risky ops' },
  { id: 'very-frequently', label: 'Very frequently', desc: 'Every minute + before risky ops' },
]

interface AutosavePrefsState {
  frequency: AutosaveFrequency
  /** When true, autosave_save fires before destructive operations
   * (e.g. delete-track, large clip removal) on top of the interval. */
  saveBeforeRiskyOps: boolean
  setFrequency: (f: AutosaveFrequency) => void
  setSaveBeforeRiskyOps: (v: boolean) => void
}

export const useAutosavePrefsStore = create<AutosavePrefsState>()(
  persist(
    (set) => ({
      // 'regularly' (5 min) matches the pre-store hardcoded 2-min cadence
      // closer than 'occasionally' (10 min) would. Users who want the
      // tighter cadence can pick 'very-frequently' (1 min).
      frequency: 'regularly',
      saveBeforeRiskyOps: true,
      setFrequency: (frequency) => set({
        frequency,
        // The 'frequently' / 'very-frequently' tiers imply the flag is on per FL docs.
        saveBeforeRiskyOps:
          frequency === 'frequently' || frequency === 'very-frequently'
            ? true
            : frequency === 'never'
              ? false
              : true,
      }),
      setSaveBeforeRiskyOps: (saveBeforeRiskyOps) => set({ saveBeforeRiskyOps }),
    }),
    { name: 'hw-autosave-prefs' },
  ),
)
