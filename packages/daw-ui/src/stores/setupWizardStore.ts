import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { invoke } from '@tauri-apps/api/core'

/**
 * The first run's state, and the MIDI input choices it and Settings > MIDI
 * make. Persists across launches so the first run only opens once.
 *
 * It keeps the setup wizard's saved answers under the same name and with
 * the same meaning, so someone who finished or skipped the wizard never
 * sees the first run:
 *
 *  - completedFirstRun: setup was finished (or skipped)
 *  - skippedAt: when the user skipped it, so it is not offered again
 *  - velocityCurves: per-input curve preset (Settings > MIDI)
 *  - midiMasterEnabled: MIDI input on or off as a whole (Settings > MIDI)
 *  - midiInputs: which inputs are on, opened again at every launch; an
 *    input in here has been seen, and one switched off stays off
 *
 * The step is transient (not persisted), so opening the first run again
 * from the Help menu always starts at the start screen.
 */

export type VelocityCurve = 'linear' | 'soft' | 'hard' | 's-curve'

/** The first run's steps: the start screen, the sound card, the
 * controller (only when one is found) and a song to start from. */
export type WizardStep = 'start' | 'sound' | 'controller' | 'song'

/** Who sees the first run: the people who never finished or skipped setup,
 *  as the saved answers say. Nobody who finished it sees it again. */
export function isFirstRun(saved: { completedFirstRun: boolean; skippedAt: number | null }): boolean {
  return !saved.completedFirstRun && saved.skippedAt == null
}

interface SetupWizardState {
  /** True while the first run is open. Not persisted: it opens by itself
   * on a first launch, and from Help > Re-run setup. */
  visible: boolean
  /** The step showing. */
  step: WizardStep

  /** Persisted: whether the user has finished the wizard at least once. */
  completedFirstRun: boolean
  /** Persisted: epoch ms when the user clicked Skip — we don't re-prompt. */
  skippedAt: number | null
  /** Master "Enable MIDI remote control" gate. When false the backend drops
   * every incoming MIDI message, so nothing reaches the audio thread. */
  midiMasterEnabled: boolean
  /** Per-input velocity curve, keyed by port name. The backend applies it to
   * incoming note-ons on that port. */
  velocityCurves: Record<string, VelocityCurve>
  /** Each MIDI input's on or off, by port name, as the wizard or the
   * settings left it. A port not in here is on (see `midiInputOn`). The
   * engine keeps no list of its own across a restart, so without this a
   * controller was dead at every launch until it was switched on again. */
  midiInputs: Record<string, boolean>

  // ── actions ──
  open: () => void
  close: () => void
  setStep: (step: WizardStep) => void
  skipForever: () => void
  markComplete: () => void
  setMidiMasterEnabled: (v: boolean) => void
  setVelocityCurve: (portName: string, curve: VelocityCurve) => void
  /** Open or close one MIDI input and remember it. Rejects with the
   * engine's reason when the port would not open; the choice is kept, and
   * the engine opens the port when the device comes back. */
  setMidiInputEnabled: (portName: string, enabled: boolean) => Promise<void>
  /** Remember an input as on or off without opening or closing it: a
   * control surface offered once stays off unless it is taken up. */
  rememberMidiInput: (portName: string, enabled: boolean) => void
}

/** Whether a MIDI input is on: what was chosen for it, or on when nothing
 * was, so a controller the wizard finds plays straight away. */
export function midiInputOn(inputs: Record<string, boolean>, portName: string): boolean {
  return inputs[portName] ?? true
}

export const useSetupWizardStore = create<SetupWizardState>()(
  persist(
    (set) => ({
      visible: false,
      step: 'start',
      completedFirstRun: false,
      skippedAt: null,
      midiMasterEnabled: true,
      velocityCurves: {},
      midiInputs: {},

      open: () => set({ visible: true, step: 'start' }),
      close: () => set({ visible: false }),
      setStep: (step) => set({ step }),
      skipForever: () =>
        set({ visible: false, skippedAt: Date.now(), completedFirstRun: true }),
      markComplete: () => set({ visible: false, completedFirstRun: true }),
      setMidiMasterEnabled: (midiMasterEnabled) => {
        set({ midiMasterEnabled })
        invoke('set_midi_master_enabled', { enabled: midiMasterEnabled }).catch(() => {})
      },
      setVelocityCurve: (portName, curve) => {
        set((s) => ({ velocityCurves: { ...s.velocityCurves, [portName]: curve } }))
        invoke('set_midi_velocity_curve', { portName, curve }).catch(() => {})
      },
      setMidiInputEnabled: async (portName, enabled) => {
        set((s) => ({ midiInputs: { ...s.midiInputs, [portName]: enabled } }))
        await invoke(enabled ? 'open_midi_input' : 'close_midi_input', { portName })
      },
      rememberMidiInput: (portName, enabled) =>
        set((s) => ({ midiInputs: { ...s.midiInputs, [portName]: enabled } })),
    }),
    {
      name: 'hw-setup-wizard',
      // step + visible are transient; everything else persists.
      partialize: (s) => ({
        completedFirstRun: s.completedFirstRun,
        skippedAt: s.skippedAt,
        midiMasterEnabled: s.midiMasterEnabled,
        velocityCurves: s.velocityCurves,
        midiInputs: s.midiInputs,
      }),
    },
  ),
)

/** Open the first run when the app boots and setup was never finished or
 * skipped. Does nothing after that. Called from `App.tsx` at boot. */
export function maybeAutoOpenSetupWizard(): void {
  const s = useSetupWizardStore.getState()
  if (isFirstRun(s)) s.open()
}

/**
 * Push the saved MIDI input settings into the backend.
 *
 * They live in localStorage, so a restart has them while the engine does not:
 * without this the master switch and every velocity curve silently went back
 * to their defaults on every launch. Called once from `App.tsx` on boot, and
 * safe to call again.
 */
export function applySavedMidiInputSettings(): void {
  const { midiMasterEnabled, velocityCurves, midiInputs } = useSetupWizardStore.getState()
  invoke('set_midi_master_enabled', { enabled: midiMasterEnabled }).catch(() => {})
  for (const [portName, curve] of Object.entries(velocityCurves)) {
    invoke('set_midi_velocity_curve', { portName, curve }).catch(() => {})
  }
  // A port that is not plugged in now is still asked for: the engine
  // keeps it wanted and opens it when the device appears.
  for (const [portName, on] of Object.entries(midiInputs)) {
    if (on) invoke('open_midi_input', { portName }).catch(() => {})
  }
}
