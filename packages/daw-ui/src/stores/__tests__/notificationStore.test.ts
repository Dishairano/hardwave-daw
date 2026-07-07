import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { useNotificationStore } from '../notificationStore'

// Pure-store tests: no Tauri, no React. This store carries every error
// the invoke layer (api/invoke.ts) surfaces, so its contract — sticky
// errors, auto-dismissing info, action buttons — is load-bearing UX.

beforeEach(() => {
  vi.useFakeTimers()
  useNotificationStore.getState().clear()
})
afterEach(() => vi.useRealTimers())

describe('notificationStore', () => {
  it('errors are sticky by default; info auto-dismisses', () => {
    const { push } = useNotificationStore.getState()
    push('error', 'save failed')
    push('info', 'exported')
    expect(useNotificationStore.getState().notifications).toHaveLength(2)

    vi.advanceTimersByTime(7000)
    const left = useNotificationStore.getState().notifications
    expect(left).toHaveLength(1)
    expect(left[0].level).toBe('error')
  })

  it('sticky override keeps an info toast alive', () => {
    useNotificationStore.getState().push('info', 'stay', { sticky: true })
    vi.advanceTimersByTime(60_000)
    expect(useNotificationStore.getState().notifications).toHaveLength(1)
  })

  it('dismiss cancels the auto-timer and removes the toast', () => {
    const id = useNotificationStore.getState().push('info', 'bye')
    useNotificationStore.getState().dismiss(id)
    expect(useNotificationStore.getState().notifications).toHaveLength(0)
    vi.advanceTimersByTime(10_000) // must not throw on a cleared timer
  })

  it('carries action buttons through to the notification', () => {
    const onClick = vi.fn()
    useNotificationStore.getState().push('error', 'failed', {
      actions: [{ label: 'Retry', onClick }],
    })
    const n = useNotificationStore.getState().notifications[0]
    expect(n.actions?.[0].label).toBe('Retry')
    n.actions?.[0].onClick()
    expect(onClick).toHaveBeenCalledOnce()
  })

  it('clear removes everything and cancels timers', () => {
    const { push, clear } = useNotificationStore.getState()
    push('info', 'a'); push('warning', 'b'); push('error', 'c')
    clear()
    expect(useNotificationStore.getState().notifications).toHaveLength(0)
  })
})
