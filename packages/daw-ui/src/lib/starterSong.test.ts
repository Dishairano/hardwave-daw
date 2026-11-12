import { describe, expect, it, vi } from 'vitest'

// The engine as it is after building the hardstyle starter: its tempo map
// says 150 BPM.
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) => {
    if (cmd === 'create_starter_song') return { genre: 'Hardstyle', bpm: 150, sections: [] }
    if (cmd === 'get_tempo_entries') return [{ tick: 0, bpm: 150, timeSigNum: 4, timeSigDen: 4, ramp: 'instant' }]
    return null
  }),
}))

const { createStarterSong } = await import('./starterSong')
const { useTempoMapStore } = await import('../stores/tempoMapStore')

describe('a starter song', () => {
  it("shows its own tempo in the tempo lane, not the empty song's 140", async () => {
    useTempoMapStore.setState({ entries: [{ tick: 0, bpm: 140, timeSigNum: 4, timeSigDen: 4, ramp: 'instant' }] })
    const song = await createStarterSong('hardstyle')
    expect(song.bpm).toBe(150)
    expect(useTempoMapStore.getState().entries.map((e) => e.bpm)).toEqual([150])
  })
})
