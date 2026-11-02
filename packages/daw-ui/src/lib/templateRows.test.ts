import { describe, expect, it } from 'vitest'
import { TEMPLATE_ROWS, insertRowId, templateRowTargets } from './templateRows'

const freshProject = () => [
  { id: 'master' },
  ...Array.from({ length: 500 }, (_, i) => ({ id: insertRowId(i + 1) })),
]

describe('a template puts its tracks where they are seen', () => {
  it('names the first playlist rows, in order', () => {
    expect(templateRowTargets(freshProject(), TEMPLATE_ROWS.vocal)).toEqual([
      { trackId: 'insert-001', name: 'Vocal' },
      { trackId: 'insert-002', name: 'Backing Vocals' },
      { trackId: 'insert-003', name: 'FX Return' },
    ])
  })

  it('gives the mixing template its ten rows, eight tracks then the buses', () => {
    const rows = templateRowTargets(freshProject(), TEMPLATE_ROWS.mixing)
    expect(rows.map((r) => r.trackId)).toEqual(Array.from({ length: 10 }, (_, i) => insertRowId(i + 1)))
    expect(rows[8].name).toBe('Bus A')
  })

  it('leaves the rest to the caller in a song without numbered rows', () => {
    expect(templateRowTargets([{ id: 'master' }, { id: 'insert-001' }], TEMPLATE_ROWS.vocal)).toEqual([
      { trackId: 'insert-001', name: 'Vocal' },
    ])
    expect(templateRowTargets([{ id: 'master' }], TEMPLATE_ROWS.vocal)).toEqual([])
  })
})
