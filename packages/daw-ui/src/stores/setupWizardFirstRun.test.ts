import { describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }))

/** localStorage with what the old setup wizard left in it. */
function storageWith(saved: Record<string, unknown> | null) {
  const items = new Map<string, string>()
  if (saved) items.set('hw-setup-wizard', JSON.stringify({ state: saved, version: 0 }))
  return {
    getItem: (k: string) => items.get(k) ?? null,
    setItem: (k: string, v: string) => { items.set(k, v) },
    removeItem: (k: string) => { items.delete(k) },
  }
}

async function bootWith(saved: Record<string, unknown> | null) {
  vi.resetModules()
  ;(globalThis as { localStorage?: unknown }).localStorage = storageWith(saved)
  const store = await import('./setupWizardStore')
  store.maybeAutoOpenSetupWizard()
  return store
}

describe('who sees the first run', () => {
  it('opens on a first launch, at the start screen', async () => {
    const { useSetupWizardStore } = await bootWith(null)
    expect(useSetupWizardStore.getState().visible).toBe(true)
    expect(useSetupWizardStore.getState().step).toBe('start')
  })

  it('does not open for someone who finished the old setup wizard', async () => {
    const { useSetupWizardStore } = await bootWith({
      completedFirstRun: true, skippedAt: null, midiMasterEnabled: true,
      velocityCurves: { 'Akai MPK Mini mk3': 'soft' }, midiInputs: { 'Akai MPK Mini mk3': true },
    })
    const s = useSetupWizardStore.getState()
    expect(s.visible).toBe(false)
    // What the wizard was told is kept.
    expect(s.velocityCurves).toEqual({ 'Akai MPK Mini mk3': 'soft' })
    expect(s.midiInputs).toEqual({ 'Akai MPK Mini mk3': true })
  })

  it('does not open for someone who skipped it', async () => {
    const { useSetupWizardStore } = await bootWith({ completedFirstRun: false, skippedAt: 1_760_000_000_000 })
    expect(useSetupWizardStore.getState().visible).toBe(false)
  })

  it('does not come back once it is finished or skipped', async () => {
    const { useSetupWizardStore, maybeAutoOpenSetupWizard } = await bootWith(null)
    useSetupWizardStore.getState().markComplete()
    useSetupWizardStore.setState({ visible: false })
    maybeAutoOpenSetupWizard()
    expect(useSetupWizardStore.getState().visible).toBe(false)

    const skipped = await bootWith(null)
    skipped.useSetupWizardStore.getState().skipForever()
    skipped.maybeAutoOpenSetupWizard()
    expect(skipped.useSetupWizardStore.getState().visible).toBe(false)
    expect(skipped.useSetupWizardStore.getState().skippedAt).not.toBeNull()
  })

  it('opens again from Help, at the start screen', async () => {
    const { useSetupWizardStore } = await bootWith({ completedFirstRun: true, skippedAt: null })
    useSetupWizardStore.setState({ step: 'song' })
    useSetupWizardStore.getState().open()
    expect(useSetupWizardStore.getState().visible).toBe(true)
    expect(useSetupWizardStore.getState().step).toBe('start')
  })
})
