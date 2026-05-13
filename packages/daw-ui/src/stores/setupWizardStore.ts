import { create } from 'zustand'
import { persist } from 'zustand/middleware'

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

export type VelocityCurve = 'linear' | 'soft' | 'hard' | 's-curve' | 'custom'

export type WizardStep = 'welcome' | 'devices' | 'velocity' | 'test' | 'done'

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
  /** Master "Enable MIDI remote control" gate. When false, no MIDI input
   * reaches the audio thread (planned wiring — currently UI-only). */
  midiMasterEnabled: boolean
  /** Per-input velocity curve preference, keyed by port name. */
  velocityCurves: Record<string, VelocityCurve>
  /** Per-input "controller type" preset (e.g. 'novation-launchkey', 'generic'). */
  controllerTypes: Record<string, string>

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
  setControllerType: (portName: string, type: string) => void
}

const STEP_ORDER: WizardStep[] = ['welcome', 'devices', 'velocity', 'test', 'done']

export const useSetupWizardStore = create<SetupWizardState>()(
  persist(
    (set, get) => ({
      visible: false,
      step: 'welcome',
      completedFirstRun: false,
      skippedAt: null,
      midiMasterEnabled: true,
      velocityCurves: {},
      controllerTypes: {},

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
      setMidiMasterEnabled: (midiMasterEnabled) => set({ midiMasterEnabled }),
      setVelocityCurve: (portName, curve) =>
        set((s) => ({ velocityCurves: { ...s.velocityCurves, [portName]: curve } })),
      setControllerType: (portName, type) =>
        set((s) => ({ controllerTypes: { ...s.controllerTypes, [portName]: type } })),
    }),
    {
      name: 'hw-setup-wizard',
      // step + visible are transient; everything else persists.
      partialize: (s) => ({
        completedFirstRun: s.completedFirstRun,
        skippedAt: s.skippedAt,
        midiMasterEnabled: s.midiMasterEnabled,
        velocityCurves: s.velocityCurves,
        controllerTypes: s.controllerTypes,
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
