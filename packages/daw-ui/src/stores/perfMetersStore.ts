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
  /** Absolute MB used by the JS heap, or null when not available. */
  memMb: number | null
  /** 0-1 ratio of usedJSHeapSize / totalJSHeapSize, or null. */
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
        // performance.memory is Chrome / Chromium / Tauri-webview only.
        let memMb: number | null = null
        let memRatio: number | null = null
        const pm = (performance as unknown as { memory?: { usedJSHeapSize: number; totalJSHeapSize: number } }).memory
        if (pm && typeof pm.usedJSHeapSize === 'number' && typeof pm.totalJSHeapSize === 'number') {
          memMb = Math.round(pm.usedJSHeapSize / (1024 * 1024))
          memRatio = pm.totalJSHeapSize > 0 ? pm.usedJSHeapSize / pm.totalJSHeapSize : null
        }
        usePerfMetersStore.getState().set({
          cpuPct: Math.round(loadPct),
          xruns,
          memMb,
          memRatio,
        })
      })
      .catch(() => { /* a missed poll is not worth a toast */ })
  }, 200)

  cleanup = () => {
    window.clearInterval(intervalId)
    cleanup = null
  }
  return cleanup
}
