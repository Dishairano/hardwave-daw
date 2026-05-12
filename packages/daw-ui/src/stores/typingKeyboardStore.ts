import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/**
 * User preference for the QWERTY-as-MIDI-piano hook
 * (`useComputerMidiKeyboard`). Persisted so toggling it survives a
 * relaunch.
 *
 * Off by default to avoid surprising new users — typing in the
 * arrangement area would otherwise spam notes through any armed/MIDI
 * track. The toolbar widget exposes a one-click toggle and the
 * persisted flag lets a returning user keep it on across sessions.
 */
export interface TypingKeyboardState {
  enabled: boolean
  setEnabled: (v: boolean) => void
  toggle: () => void
}

export const useTypingKeyboardStore = create<TypingKeyboardState>()(
  persist(
    (set) => ({
      enabled: false,
      setEnabled: (enabled: boolean) => set({ enabled }),
      toggle: () => set((s) => ({ enabled: !s.enabled })),
    }),
    { name: 'hw-typing-keyboard' },
  ),
)
