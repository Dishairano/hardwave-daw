// The app's way to the backend. vite.config.ts points every import of
// '@tauri-apps/api/core' here, so the FPS meter can time the calls. It
// used to wrap Tauri's own invoke on window.__TAURI_INTERNALS__, which is
// read-only in the app: turning the meter on broke the whole interface.
import * as core from 'tauri-api-core-original'
import { noteBackendCall, timingBackendCalls } from '../services/frameStats'
import { readKey, shareRead } from './sharedReads'

export * from 'tauri-api-core-original'

// A poll that is still waiting for its last answer shares it (see sharedReads).
export function invoke<T>(cmd: string, args?: core.InvokeArgs, options?: core.InvokeOptions): Promise<T> {
  return shareRead(readKey(cmd, args, options), () => timedInvoke<T>(cmd, args, options))
}

async function timedInvoke<T>(cmd: string, args?: core.InvokeArgs, options?: core.InvokeOptions): Promise<T> {
  if (!timingBackendCalls()) return core.invoke<T>(cmd, args, options)
  const started = performance.now()
  try {
    return await core.invoke<T>(cmd, args, options)
  } finally {
    noteBackendCall(cmd, performance.now() - started)
  }
}
