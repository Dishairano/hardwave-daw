import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { invoke } from '@tauri-apps/api/core'

/**
 * First-run Setup Wizard state. Tracks which steps the user has
 * completed, per-device velocity curve preferences, and the master
 * "Enable MIDI remote control" gate. Persists across launches so the
 * wizard only auto-opens once.
 *
 * The wizard wraps the existing MIDI Tauri commands (list_midi_inputs
 * / open_midi_input / close_midi_input / get_midi_activity) — those
 * are the source of truth for port state. This store captures pure
 * preferences the backend doesn't track:
 *
 *  - completedFirstRun: dismisses the auto-open
 *  - skippedAt: remembers if the user hit "Skip — won't ask again"
 *  - velocityCurves: per-input curve preset
 *  - midiMasterEnabled: master gate (future: gates events at engine level)
 *
 * Wizard step is transient (not persisted) so reopening the wizard
 * always starts from step 1.
 */

export type VelocityCurve = 'linear' | 'soft' | 'hard' | 's-curve'

export type WizardStep = 'welcome' | 'audio' | 'devices' | 'velocity' | 'test' | 'done'

interface SetupWizardState {
  /** True when the wizard is open (modal visible). Not persisted — opening
   * is via auto-trigger on first launch OR Help menu → "Re-run setup wizard". */
  visible: boolean
  /** Active step in the 5-step flow. */
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

  // ── actions ──
  open: () => void
  close: () => void
  setStep: (step: WizardStep) => void
  next: () => void
  back: () => void
  skipForever: () => void
  markComplete: () => void
  setMidiMasterEnabled: (v: boolean) => void
  setVelocityCurve: (portName: string, curve: VelocityCurve) => void
}

const STEP_ORDER: WizardStep[] = ['welcome', 'audio', 'devices', 'velocity', 'test', 'done']

export const useSetupWizardStore = create<SetupWizardState>()(
  persist(
    (set, get) => ({
      visible: false,
      step: 'welcome',
      completedFirstRun: false,
      skippedAt: null,
      midiMasterEnabled: true,
      velocityCurves: {},

      open: () => set({ visible: true, step: 'welcome' }),
      close: () => set({ visible: false }),
      setStep: (step) => set({ step }),
      next: () => {
        const idx = STEP_ORDER.indexOf(get().step)
        const nextStep = STEP_ORDER[Math.min(STEP_ORDER.length - 1, idx + 1)]
        set({ step: nextStep })
      },
      back: () => {
        const idx = STEP_ORDER.indexOf(get().step)
        const prev = STEP_ORDER[Math.max(0, idx - 1)]
        set({ step: prev })
      },
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
    }),
    {
      name: 'hw-setup-wizard',
      // step + visible are transient; everything else persists.
      partialize: (s) => ({
        completedFirstRun: s.completedFirstRun,
        skippedAt: s.skippedAt,
        midiMasterEnabled: s.midiMasterEnabled,
        velocityCurves: s.velocityCurves,
      }),
    },
  ),
)

/** Auto-open the wizard once when the app first boots and the user
 * hasn't completed or skipped it yet. Idempotent — subsequent calls
 * do nothing once the flag is set. Called from `App.tsx` setup hook. */
export function maybeAutoOpenSetupWizard(): void {
  const s = useSetupWizardStore.getState()
  if (!s.completedFirstRun && s.skippedAt == null) {
    s.open()
  }
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
  const { midiMasterEnabled, velocityCurves } = useSetupWizardStore.getState()
  invoke('set_midi_master_enabled', { enabled: midiMasterEnabled }).catch(() => {})
  for (const [portName, curve] of Object.entries(velocityCurves)) {
    invoke('set_midi_velocity_curve', { portName, curve }).catch(() => {})
  }
}
