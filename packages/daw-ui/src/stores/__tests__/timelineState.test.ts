import { describe, it, expect, beforeEach, vi } from 'vitest'

const invokeMock = vi.fn().mockResolvedValue(null)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }))

// jsdom is not configured for these store tests, so stand in for the browser
// storage the legacy path reads.
const store = new Map<string, string>()
vi.stubGlobal('localStorage', {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
  removeItem: (k: string) => void store.delete(k),
  clear: () => store.clear(),
})

const { useMarkerStore } = await import('../markerStore')
const { useTransportStore } = await import('../transportStore')
const {
  parseTimelineState,
  serializeTimelineState,
  hydrateTimelineState,
  resetTimelineState,
} = await import('../timelineState')

const marker = (id: string, tick: number) => ({
  id, tick, label: id, color: '#fff', kind: 'generic' as const,
})

beforeEach(() => {
  store.clear()
  invokeMock.mockClear()
  useMarkerStore.setState({ markers: [] })
  useTransportStore.setState({ punchEnabled: false, punchInTicks: null, punchOutTicks: null })
})

describe('parseTimelineState', () => {
  it('drops malformed markers instead of failing the whole project load', () => {
    const parsed = parseTimelineState(JSON.stringify({
      markers: [marker('good', 480), { id: 'bad', tick: 'nope' }, null],
      punch: { enabled: true, inTicks: 960, outTicks: 1920 },
    }))
    expect(parsed?.markers.map(m => m.id)).toEqual(['good'])
    expect(parsed?.punch).toEqual({ enabled: true, inTicks: 960, outTicks: 1920 })
  })

  it('returns null for absent or unparseable state', () => {
    expect(parseTimelineState(null)).toBeNull()
    expect(parseTimelineState('not json')).toBeNull()
    expect(parseTimelineState('[]')).toEqual({
      markers: [],
      punch: { enabled: false, inTicks: null, outTicks: null },
    })
  })

  it('rejects negative punch positions', () => {
    const parsed = parseTimelineState(JSON.stringify({
      markers: [], punch: { enabled: true, inTicks: -5, outTicks: 100 },
    }))
    expect(parsed?.punch.inTicks).toBeNull()
    expect(parsed?.punch.outTicks).toBe(100)
  })
})

describe('hydrateTimelineState', () => {
  it('loads a project\'s markers and punch range into the stores', () => {
    hydrateTimelineState(JSON.stringify({
      markers: [marker('b', 1920), marker('a', 480)],
      punch: { enabled: true, inTicks: 480, outTicks: 1920 },
    }))
    expect(useMarkerStore.getState().markers.map(m => m.id)).toEqual(['a', 'b'])
    expect(useTransportStore.getState().punchInTicks).toBe(480)
    expect(useTransportStore.getState().punchEnabled).toBe(true)
  })

  it('clears the previous song\'s timeline when a project carries none', () => {
    useMarkerStore.setState({ markers: [marker('stale', 100)] })
    useTransportStore.setState({ punchEnabled: true, punchInTicks: 10, punchOutTicks: 20 })
    hydrateTimelineState(null)
    expect(useMarkerStore.getState().markers).toEqual([])
    expect(useTransportStore.getState().punchInTicks).toBeNull()
    expect(useTransportStore.getState().punchEnabled).toBe(false)
  })

  it('adopts markers left in localStorage once, then forgets them', () => {
    store.set('hardwave.daw.markers', JSON.stringify([marker('old', 720)]))
    store.set('hardwave.daw.punchIn', '240')
    store.set('hardwave.daw.punchEnabled', '1')

    hydrateTimelineState(null)
    expect(useMarkerStore.getState().markers.map(m => m.id)).toEqual(['old'])
    expect(useTransportStore.getState().punchInTicks).toBe(240)
    expect(store.has('hardwave.daw.markers')).toBe(false)

    // Second project without timeline state gets a clean timeline, because
    // the legacy keys are gone.
    hydrateTimelineState(null)
    expect(useMarkerStore.getState().markers).toEqual([])
  })

  it('prefers the project over anything still in localStorage', () => {
    store.set('hardwave.daw.markers', JSON.stringify([marker('old', 720)]))
    hydrateTimelineState(JSON.stringify({ markers: [marker('fromProject', 60)] }))
    expect(useMarkerStore.getState().markers.map(m => m.id)).toEqual(['fromProject'])
    expect(store.has('hardwave.daw.markers')).toBe(true)
  })
})

describe('serializeTimelineState', () => {
  it('round-trips what the stores hold', () => {
    useMarkerStore.getState().setMarkers([marker('m1', 960)])
    useTransportStore.setState({ punchEnabled: true, punchInTicks: 12, punchOutTicks: 48 })
    const parsed = parseTimelineState(serializeTimelineState())
    expect(parsed?.markers.map(m => m.id)).toEqual(['m1'])
    expect(parsed?.punch).toEqual({ enabled: true, inTicks: 12, outTicks: 48 })
  })

  it('resetTimelineState empties both', () => {
    useMarkerStore.getState().setMarkers([marker('m1', 960)])
    useTransportStore.setState({ punchEnabled: true, punchInTicks: 12, punchOutTicks: 48 })
    resetTimelineState()
    expect(useMarkerStore.getState().markers).toEqual([])
    expect(useTransportStore.getState().punchOutTicks).toBeNull()
  })
})
