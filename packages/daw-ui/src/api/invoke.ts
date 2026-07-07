/**
 * Typed Tauri invoke wrappers that surface failures to the user.
 *
 * The raw `invoke()` pattern scattered across the app swallowed errors
 * into `console.error` (or empty catches) — a failed save/undo/param-set
 * looked exactly like success (deep-research P1-8). Route calls whose
 * failure the USER must know about through these wrappers; fire-and-forget
 * cosmetics (hover info, meter polls) can stay on raw invoke.
 */
import { invoke } from '@tauri-apps/api/core'
import { useNotificationStore } from '../stores/notificationStore'

type Args = Record<string, unknown>

/**
 * Invoke that shows an error toast (with Retry) on failure and rethrows,
 * so callers can still abort their own flow.
 */
export async function invokeOrToast<T>(
  cmd: string,
  args?: Args,
  opts?: { message?: string; retry?: boolean },
): Promise<T> {
  try {
    return await invoke<T>(cmd, args)
  } catch (e) {
    const push = useNotificationStore.getState().push
    push('error', opts?.message ?? `Operation failed (${cmd.replace(/_/g, ' ')})`, {
      detail: String(e),
      actions:
        opts?.retry === false
          ? undefined
          : [{ label: 'Retry', onClick: () => { void invokeOrToast<T>(cmd, args, opts) } }],
    })
    throw e
  }
}

/**
 * Like `invokeOrToast` but resolves to `null` instead of rethrowing —
 * for callers with nothing further to unwind.
 */
export async function invokeOrNull<T>(
  cmd: string,
  args?: Args,
  opts?: { message?: string; retry?: boolean },
): Promise<T | null> {
  try {
    return await invokeOrToast<T>(cmd, args, opts)
  } catch {
    return null
  }
}
