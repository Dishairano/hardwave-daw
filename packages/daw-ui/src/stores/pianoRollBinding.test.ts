import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }))

const { useTrackStore } = await import('./trackStore')
const { followSelectionInPianoRoll } = await import('./pianoRollBinding')

const track = (id: string, kind: string) => ({ id, kind, name: id, clips: [] }) as unknown as ReturnType<typeof useTrackStore.getState>['tracks'][number]
const settle = () => new Promise(r => setTimeout(r, 0))

describe('the piano roll, open with nothing in it', () => {
  beforeEach(() => {
    useTrackStore.setState({
      tracks: [track('insert-001', 'Audio'), track('lead', 'Midi')],
      selectedTrackId: null,
      activeMidiTrackId: null,
      activeMidiClipId: null,
      ensureMidiClipOnTrack: async (id: string) => `clip-of-${id}`,
    })
  })

  it('takes the instrument channel picked after it opened', async () => {
    const stop = followSelectionInPianoRoll()
    await settle()
    expect(useTrackStore.getState().activeMidiClipId).toBeNull()
    useTrackStore.getState().selectTrack('lead')
    await settle()
    expect(useTrackStore.getState().activeMidiTrackId).toBe('lead')
    expect(useTrackStore.getState().activeMidiClipId).toBe('clip-of-lead')
    stop()
  })

  it('leaves an audio track alone, and stops following when closed', async () => {
    const stop = followSelectionInPianoRoll()
    useTrackStore.getState().selectTrack('insert-001')
    await settle()
    expect(useTrackStore.getState().activeMidiClipId).toBeNull()
    stop()
    useTrackStore.getState().selectTrack('lead')
    await settle()
    expect(useTrackStore.getState().activeMidiClipId).toBeNull()
  })
})
