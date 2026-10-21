// One request at a time for the calls the interface polls.
//
// The interface reads meters and states from the backend on timers, about 24
// calls a second with the DAW idle. On Windows each call is a request that the
// app's main thread answers, and a timer does not wait for the last answer: when
// the main thread is busy, unanswered requests pile up, and each holds about
// 60 KB in WebView2's browser and network processes until it is answered. On
// the test VM under load that came to 12 GB in two hours (20 Oct 2026); with the
// main thread paused for 90 s, the two processes grew by 140 MB and fell back
// once it answered.
//
// So while a call that only reads is unanswered, the same call with the same
// arguments shares its answer instead of sending another request.

import type { InvokeArgs, InvokeOptions } from '@tauri-apps/api/core'

/** Calls that only read, besides every `get_*` command. */
const READS = new Set(['process_memory', 'midi_learn_status', 'collab_status', 'list_midi_mappings'])

const pending = new Map<string, Promise<unknown>>()

/** What identifies a call for sharing, or `null` when it must always go out on its own. */
export function readKey(cmd: string, args?: InvokeArgs, options?: InvokeOptions): string | null {
  if (options) return null
  if (!cmd.startsWith('get_') && !READS.has(cmd)) return null
  try {
    return `${cmd}\u0000${JSON.stringify(args ?? null)}`
  } catch {
    return null
  }
}

/** Starts the call, or hands back the answer still on its way for the same call. */
export function shareRead<T>(key: string | null, start: () => Promise<T>): Promise<T> {
  if (key === null) return start()
  const waiting = pending.get(key)
  if (waiting) return waiting as Promise<T>
  const call = start().finally(() => pending.delete(key))
  pending.set(key, call)
  return call
}
