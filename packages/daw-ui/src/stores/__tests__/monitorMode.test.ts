import { describe, it, expect, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }))

const { monitorModeOf, nextMonitorMode } = await import('../trackStore')

describe('monitoring modes', () => {
  it('reads the mode from the two flags the engine keeps', () => {
    expect(monitorModeOf({ monitorInput: false, monitorAuto: true })).toBe('off')
    expect(monitorModeOf({ monitorInput: true, monitorAuto: false })).toBe('in')
    expect(monitorModeOf({ monitorInput: true })).toBe('in')
    expect(monitorModeOf({ monitorInput: true, monitorAuto: true })).toBe('auto')
  })

  it('the button cycles Off, In, Auto and back', () => {
    expect(nextMonitorMode('off')).toBe('in')
    expect(nextMonitorMode('in')).toBe('auto')
    expect(nextMonitorMode('auto')).toBe('off')
  })
})
