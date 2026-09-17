import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

/**
 * Performance-meter source for the toolbar's CPU / RAM cluster.
 *
 *  - CPU: real audio-thread load, read from the engine. It is the share of
 *         each audio block's time budget the engine spends doing the work:
 *         at 48 kHz with 256-frame blocks the budget is 5.33 ms, so 2.6 ms
 *         of work reads 50%, and going past 100% means the device went
 *         unfed and the user heard a click. Those overruns are counted, and
 *         the count is what the tooltip warns about, because an average
 *         that looks calm with a rising xrun count is still a broken
 *         session.
 *
 *         This used to estimate load from the gap between animation frames,
 *         which measures how busy the WebView is: it climbed while a window
 *         was dragged and sat still while the audio thread was close to
 *         dropping out. A producer deciding whether to add one more plug-in
 *         was reading a number about the user interface.
 *
 *  - MEM: pulled from `performance.memory.usedJSHeapSize`, available
 *         on Chromium / Tauri's webview. Falls back to `null` on
 *         engines that don't expose it (the toolbar then shows an
 *         em-dash). We track usedJSHeapSize / totalJSHeapSize for a
 *         ratio, plus the absolute MB for the tooltip.
 *
 * Both metrics update at ~5 Hz (every 200 ms) — high enough to feel
 * live without burning the main thread on render work.
 */

interface PerfMetersState {
  /** 0-100+, share of the audio block budget in use. Over 100 = dropouts. */
  cpuPct: number
  /** Audio blocks that overran their budget this session. */
  xruns: number
  /** MB this process is using, or null when it cannot be read. */
  memMb: number | null
  /** 0-1 share of the machine's physical memory, or null. */
  memRatio: number | null
  set: (next: { cpuPct: number; xruns: number; memMb: number | null; memRatio: number | null }) => void
}

export const usePerfMetersStore = create<PerfMetersState>((set) => ({
  cpuPct: 0,
  xruns: 0,
  memMb: null,
  memRatio: null,
  set: (next) => set(next),
}))

/**
 * Boot the meter sampler — call once from the top-level App. Cleans up
 * its rAF + interval handles when invoked again or on unmount.
 */
let cleanup: (() => void) | null = null
export function startPerfMeters(): () => void {
  if (cleanup) cleanup()

  const intervalId = window.setInterval(() => {
    // The engine owns this number: it is measured on the audio thread, per
    // block, against that block's real deadline.
    invoke<{ loadPct: number; xruns: number }>('get_audio_load')
      .then(({ loadPct, xruns }) => {
        const { memMb, memRatio } = usePerfMetersStore.getState()
        usePerfMetersStore.getState().set({
          cpuPct: Math.round(loadPct),
          xruns,
          memMb,
          memRatio,
        })
      })
      .catch(() => { /* a missed poll is not worth a toast */ })
  }, 200)

  // Memory every second rather than five times a second: it moves slowly,
  // and it is a syscall rather than a read of a value the engine already has.
  const memoryId = window.setInterval(pollMemory, 1000)
  pollMemory()

  cleanup = () => {
    window.clearInterval(intervalId)
    window.clearInterval(memoryId)
    cleanup = null
  }
  return cleanup
}

/**
 * Read this process's memory use.
 *
 * The meter used to read `performance.memory.usedJSHeapSize`, the WebView's
 * JavaScript heap. Nothing that costs real memory in a DAW lives there: the
 * sample pool, the audio graph and every loaded plug-in are in the Rust
 * process, so a project holding gigabytes of samples showed tens of
 * megabytes. The share is now of the machine's physical memory, which is what
 * a producer would compare against.
 */
function pollMemory(): void {
  invoke<{ usedBytes: number | null; totalBytes: number | null }>('process_memory')
    .then(({ usedBytes, totalBytes }) => {
      const state = usePerfMetersStore.getState()
      state.set({
        cpuPct: state.cpuPct,
        xruns: state.xruns,
        memMb: usedBytes == null ? null : Math.round(usedBytes / (1024 * 1024)),
        memRatio:
          usedBytes != null && totalBytes != null && totalBytes > 0
            ? usedBytes / totalBytes
            : null,
      })
    })
    .catch(() => { /* a missed poll is not worth a toast */ })
}
