import { describe, it, expect, vi, beforeEach } from 'vitest'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useTrackStore } = await import('../trackStore')

function trackWithLanes(lanes: unknown[]) {
  return [{ id: 't1', name: 'T1', kind: 'Audio', clips: [], automationLanes: lanes }]
}

describe('ensureAutomationLane', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockResolvedValue([])
    useTrackStore.setState({ tracks: trackWithLanes([]) as never, tracksById: {} as never })
  })

  it('creates a lane when the target has none', async () => {
    invokeMock.mockResolvedValueOnce('lane-new').mockResolvedValueOnce([])
    const id = await useTrackStore.getState().ensureAutomationLane('t1', { kind: 'track_volume' })

    expect(id).toBe('lane-new')
    expect(invokeMock).toHaveBeenCalledWith('add_automation_lane', {
      trackId: 't1',
      target: { kind: 'track_volume' },
    })
  })

  it('reuses the lane that already targets it', async () => {
    useTrackStore.setState({
      tracks: trackWithLanes([
        { id: 'lane-1', target: { kind: 'track_volume' }, points: [], visible: true },
      ]) as never,
    })
    const id = await useTrackStore.getState().ensureAutomationLane('t1', { kind: 'track_volume' })

    expect(id).toBe('lane-1')
    expect(invokeMock).not.toHaveBeenCalledWith('add_automation_lane', expect.anything())
  })

  it('shows a lane again when it was hidden', async () => {
    useTrackStore.setState({
      tracks: trackWithLanes([
        { id: 'lane-1', target: { kind: 'track_volume' }, points: [], visible: false },
      ]) as never,
    })
    await useTrackStore.getState().ensureAutomationLane('t1', { kind: 'track_volume' })

    expect(invokeMock).toHaveBeenCalledWith('set_automation_lane_visible', {
      trackId: 't1',
      laneId: 'lane-1',
      visible: true,
    })
  })

  it('tells a plug-in parameter lane apart from another parameter on the same plug-in', async () => {
    useTrackStore.setState({
      tracks: trackWithLanes([
        {
          id: 'lane-cutoff',
          target: { kind: 'plugin_param', slotId: 's1', paramId: 7 },
          points: [],
          visible: true,
        },
      ]) as never,
    })
    invokeMock.mockResolvedValueOnce('lane-res').mockResolvedValueOnce([])

    const same = await useTrackStore
      .getState()
      .ensureAutomationLane('t1', { kind: 'plugin_param', slotId: 's1', paramId: 7 })
    expect(same).toBe('lane-cutoff')

    const other = await useTrackStore
      .getState()
      .ensureAutomationLane('t1', { kind: 'plugin_param', slotId: 's1', paramId: 8 })
    expect(other).toBe('lane-res')
  })
})
