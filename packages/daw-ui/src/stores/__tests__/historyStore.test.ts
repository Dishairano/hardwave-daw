import { describe, it, expect, beforeEach } from 'vitest'
import { useHistoryStore } from '../historyStore'

// The frontend history mirrors the Rust undo stack (labels only). Its
// cursor discipline is what keeps the History panel truthful — a desync
// here shows users wrong undo labels even when Rust is correct.

beforeEach(() => useHistoryStore.getState().clear())

describe('historyStore', () => {
  it('push after undo truncates the redo branch', () => {
    const s = () => useHistoryStore.getState()
    s().push('add track')
    s().push('move clip')
    s().undoOne()
    expect(s().cursor).toBe(1)
    s().push('delete clip') // new branch — redo history must vanish
    expect(s().entries.map(e => e.label)).toEqual(['add track', 'delete clip'])
    expect(s().cursor).toBe(2)
  })

  it('undo/redo clamp at the ends', () => {
    const s = () => useHistoryStore.getState()
    s().push('a')
    s().undoOne(); s().undoOne(); s().undoOne()
    expect(s().cursor).toBe(0)
    s().redoOne(); s().redoOne(); s().redoOne()
    expect(s().cursor).toBe(1)
  })

  it('caps entries at 200, dropping the oldest', () => {
    const s = () => useHistoryStore.getState()
    for (let i = 0; i < 205; i++) s().push(`edit ${i}`)
    expect(s().entries).toHaveLength(200)
    expect(s().entries[0].label).toBe('edit 5')
    expect(s().cursor).toBe(200)
  })

  it('jumpTo walks undo/redo callbacks the right number of times', async () => {
    // Contract: jumpTo does NOT move the cursor itself — the callbacks
    // do (production passes trackStore.undo/redo, which call
    // undoOne/redoOne on success). The test callbacks mirror that.
    const s = () => useHistoryStore.getState()
    s().push('a'); s().push('b'); s().push('c')
    const calls: string[] = []
    const undo = async () => { calls.push('undo'); s().undoOne(); return true }
    const redo = async () => { calls.push('redo'); s().redoOne(); return true }
    await s().jumpTo(1, undo, redo)
    expect(calls).toEqual(['undo', 'undo'])
    expect(s().cursor).toBe(1)
    await s().jumpTo(3, undo, redo)
    expect(calls).toEqual(['undo', 'undo', 'redo', 'redo'])
    expect(s().cursor).toBe(3)
  })

  it('jumpTo stops early when a step fails (Rust rejected the undo)', async () => {
    const s = () => useHistoryStore.getState()
    s().push('a'); s().push('b'); s().push('c')
    let allowed = 1
    const undo = async () => {
      if (allowed-- > 0) { s().undoOne(); return true }
      return false
    }
    await s().jumpTo(0, undo, async () => true)
    // One undo succeeded, second failed → cursor reflects reality (2),
    // not the requested target (0) — the History panel stays truthful.
    expect(s().cursor).toBe(2)
  })
})
