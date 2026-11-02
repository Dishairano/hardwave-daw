import { describe, it, expect, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: () => Promise.resolve(null) }))
// The language store keeps its choice in browser storage.
vi.stubGlobal('localStorage', {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
})

const { joinedNotice } = await import('../collabStore')

describe('what a guest is told on coming into a room', () => {
  it('points a first visit at the song, even when the host has worked a while', () => {
    // The room kept more than it can replay: caught up is false.
    expect(joinedNotice('K7QM2XPA', false)).toBe('Get the song from them to start from what they have.')
  })

  it('says what the replay did when coming back to the same room', () => {
    expect(joinedNotice('ROOMB', true)).toMatch(/Get the song/)
    expect(joinedNotice('ROOMB', false)).toBe('You were away too long to replay: ask them to send the project.')
    expect(joinedNotice('ROOMB', true)).toBe('Everything you missed has been applied.')
  })
})
