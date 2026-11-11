import { describe, it, expect, vi, beforeEach } from 'vitest'

// The History panel's list is the engine's undo stack. These tests stand
// in for the engine: a stack of labels and a cursor that undo and redo
// move, as get_undo_history reports them.
let engine: { labels: string[]; cursor: number }
const invokeMock = vi.fn(async (cmd: string, _args?: Record<string, unknown>) => {
  if (cmd === 'get_undo_history') {
    return { entries: engine.labels.map((label, i) => ({ label, time: i })), cursor: engine.cursor, startsAtOpen: true }
  }
  if (cmd === 'label_undo_step') return true
  return null
})
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => invokeMock(cmd, args),
}))

// The engine's events: what the History panel listens to.
const listeners = new Map<string, () => void>()
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, handler: () => void) => {
    listeners.set(name, handler)
    return () => { listeners.delete(name) }
  },
}))

const { useHistoryStore } = await import('../historyStore')

beforeEach(() => {
  invokeMock.mockClear()
  engine = { labels: [], cursor: 0 }
  useHistoryStore.setState({ entries: [], cursor: 0 })
})

describe('historyStore', () => {
  it('shows the engine list, with its cursor', async () => {
    engine = { labels: ['Add track "Kick"', 'Add {n} notes: "{name}"'], cursor: 1 }
    await useHistoryStore.getState().refresh()
    const s = useHistoryStore.getState()
    expect(s.entries.map(e => e.label)).toEqual(['Add track "Kick"', 'Add {n} notes: "{name}"'])
    expect(s.cursor).toBe(1)
  })

  it('a label goes to the engine, which decides whether it names a step', () => {
    useHistoryStore.getState().push('Mute clip', 1234)
    expect(invokeMock).toHaveBeenCalledWith('label_undo_step', { label: 'Mute clip', since: 1234 })
    // The list itself is not touched: only the engine adds rows.
    expect(useHistoryStore.getState().entries).toHaveLength(0)
  })

  it('jumpTo walks undo and redo the right number of times', async () => {
    engine = { labels: ['a', 'b', 'c'], cursor: 3 }
    const calls: string[] = []
    const undo = async () => { calls.push('undo'); engine.cursor--; return true }
    const redo = async () => { calls.push('redo'); engine.cursor++; return true }
    await useHistoryStore.getState().jumpTo(1, undo, redo)
    expect(calls).toEqual(['undo', 'undo'])
    expect(useHistoryStore.getState().cursor).toBe(1)
    await useHistoryStore.getState().jumpTo(3, undo, redo)
    expect(calls).toEqual(['undo', 'undo', 'redo', 'redo'])
    expect(useHistoryStore.getState().cursor).toBe(3)
  })

  it('jumpTo stops when a step fails and shows where the engine is', async () => {
    engine = { labels: ['a', 'b', 'c'], cursor: 3 }
    let allowed = 1
    const undo = async () => {
      if (allowed-- > 0) { engine.cursor--; return true }
      return false
    }
    await useHistoryStore.getState().jumpTo(0, undo, async () => true)
    expect(useHistoryStore.getState().cursor).toBe(2)
  })

  it('reads the list when the engine says its steps changed, and not on a timer', async () => {
    vi.useFakeTimers()
    try {
      const stop = useHistoryStore.getState().follow()
      await vi.waitFor(() => expect(listeners.has('daw:historyChanged')).toBe(true))
      await vi.advanceTimersByTimeAsync(5000)
      expect(invokeMock).not.toHaveBeenCalledWith('get_undo_history', undefined)
      engine = { labels: ['Volume: "{name}"'], cursor: 1 }
      listeners.get('daw:historyChanged')!()
      await vi.waitFor(() => expect(useHistoryStore.getState().entries).toHaveLength(1))
      stop()
      expect(listeners.has('daw:historyChanged')).toBe(false)
    } finally {
      vi.useRealTimers()
    }
  })

  it('asked many times at once, reads the list once more after the read on its way', async () => {
    engine = { labels: ['a'], cursor: 1 }
    const reads = [1, 2, 3, 4, 5].map(() => useHistoryStore.getState().refresh())
    engine = { labels: ['a', 'b'], cursor: 2 }
    await Promise.all(reads)
    const calls = invokeMock.mock.calls.filter(([cmd]) => cmd === 'get_undo_history')
    expect(calls).toHaveLength(2)
    expect(useHistoryStore.getState().entries.map(e => e.label)).toEqual(['a', 'b'])
  })
})
