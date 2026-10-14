import { describe, it, expect, beforeEach } from 'vitest'
import { useNotificationStore } from '../notificationStore'

describe('sticky notifications', () => {
  beforeEach(() => useNotificationStore.setState({ notifications: [] }))

  it('the same sticky message again is kept once, not stacked', () => {
    const push = useNotificationStore.getState().push
    const first = push('warning', 'Audio device lost — switched to Speakers', { sticky: true, detail: 'one' })
    const again = push('warning', 'Audio device lost — switched to Speakers', { sticky: true, detail: 'two' })
    const list = useNotificationStore.getState().notifications
    expect(again).toBe(first)
    expect(list).toHaveLength(1)
    expect(list[0].detail).toBe('two')
    // A different message, or the same one not sticky, still shows on its own.
    push('warning', 'Audio device lost — switched to Headphones', { sticky: true })
    push('info', 'Audio device lost — switched to Speakers')
    expect(useNotificationStore.getState().notifications).toHaveLength(3)
  })
})
