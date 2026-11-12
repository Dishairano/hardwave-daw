import { beforeEach, describe, expect, it, vi } from 'vitest'

const calls: Array<[string, Record<string, unknown> | undefined]> = []
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push([cmd, args])
    if (cmd === 'get_tracks_with_clips') return []
    return null
  }),
}))

const { autoCrossfadeMenuItem } = await import('./autoCrossfadeMenu')
const { overlapFades } = await import('../../stores/autoCrossfade')
const { useTrackStore } = await import('../../stores/trackStore')
const { useNotificationStore } = await import('../../stores/notificationStore')

const clip = (id: string, at: number, len: number, extra: Record<string, unknown> = {}) => ({
  id, kind: 'audio', position_ticks: at, length_ticks: len, fadeInTicks: 0, fadeOutTicks: 0, row: 0, ...extra,
})

describe('auto-crossfade', () => {
  it('fades the earlier clip out and the later one in over their overlap', () => {
    const { edits, overlaps } = overlapFades([clip('b', 900, 960), clip('a', 0, 960)])
    expect(overlaps).toBe(1)
    expect(edits).toEqual([
      { clipId: 'a', fadeInTicks: 0, fadeOutTicks: 60 },
      { clipId: 'b', fadeInTicks: 60, fadeOutTicks: 0 },
    ])
  })

  it('gives a clip between two overlaps both fades', () => {
    const { edits } = overlapFades([clip('a', 0, 960), clip('b', 900, 960), clip('c', 1800, 960)])
    expect(edits.find((e) => e.clipId === 'b')).toEqual({ clipId: 'b', fadeInTicks: 60, fadeOutTicks: 60 })
  })

  it('leaves clips on other rows, MIDI clips and clips that only touch alone', () => {
    const { edits, overlaps } = overlapFades([
      clip('a', 0, 960), clip('b', 480, 960, { row: 1 }),
      clip('m', 0, 960, { kind: 'midi' }), clip('n', 480, 960, { kind: 'midi' }),
      clip('c', 960, 960),
    ])
    expect(overlaps).toBe(0)
    expect(edits).toEqual([])
  })
})

describe('Edit > Auto-crossfade overlaps', () => {
  beforeEach(() => {
    calls.length = 0
    useNotificationStore.setState({ notifications: [] })
  })

  it('crossfades every overlap in the song as one undo step and says how many', async () => {
    useTrackStore.setState({
      tracks: [
        { id: 'master', kind: 'Master', clips: [] },
        { id: 'vox', kind: 'Audio', clips: [clip('a', 0, 960), clip('b', 900, 960)] },
      ] as never,
    })
    const item = autoCrossfadeMenuItem()
    expect(item.label).toBe('Auto-crossfade overlaps')
    item.action?.()
    await vi.waitFor(() => expect(useNotificationStore.getState().notifications.length).toBe(1))
    // Both fades inside one history group: one undo step.
    expect(calls.slice(0, 4).map(([cmd]) => cmd)).toEqual([
      'begin_history_group', 'set_clip_fades', 'set_clip_fades', 'end_history_group',
    ])
    expect(calls[1][1]).toEqual({ trackId: 'vox', clipId: 'a', fadeInTicks: 0, fadeOutTicks: 60 })
    expect(calls[2][1]).toEqual({ trackId: 'vox', clipId: 'b', fadeInTicks: 60, fadeOutTicks: 0 })
    expect(calls.some(([cmd]) => cmd === 'get_tracks_with_clips')).toBe(true)
    expect(useNotificationStore.getState().notifications[0].message).toBe('Crossfaded 1 overlap')
  })

  it('says so when nothing overlaps', async () => {
    useTrackStore.setState({ tracks: [{ id: 'vox', kind: 'Audio', clips: [clip('a', 0, 960)] }] as never })
    autoCrossfadeMenuItem().action?.()
    await vi.waitFor(() => expect(useNotificationStore.getState().notifications.length).toBe(1))
    expect(calls).toEqual([])
    expect(useNotificationStore.getState().notifications[0].message).toBe('No overlapping audio clips to crossfade')
  })
})
