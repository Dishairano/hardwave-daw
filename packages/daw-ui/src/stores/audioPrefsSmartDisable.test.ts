import { describe, expect, it, vi } from 'vitest'

const calls: Array<[string, unknown]> = []
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args: unknown) => {
    calls.push([cmd, args])
    return null
  }),
}))

const { useAudioPrefsStore, applySavedAudioPrefs } = await import('./audioPrefsStore')

describe('smart disable in Settings > Audio', () => {
  it('is on by default and reaches the engine on boot', () => {
    expect(useAudioPrefsStore.getState().smartDisable).toBe(true)
    calls.length = 0
    applySavedAudioPrefs()
    expect(calls).toContainEqual(['set_smart_disable', { enabled: true }])
  })

  it('tells the engine when it is switched', () => {
    calls.length = 0
    useAudioPrefsStore.getState().setSmartDisable(false)
    expect(useAudioPrefsStore.getState().smartDisable).toBe(false)
    expect(calls).toContainEqual(['set_smart_disable', { enabled: false }])
  })
})
