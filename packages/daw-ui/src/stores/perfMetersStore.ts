import { create } from 'zustand'

/**
 * Lightweight performance-meter source for the toolbar's RAM / CPU
 * cluster.
 *
 *  - CPU: frame-time based estimate. We sample the gap between
 *         `requestAnimationFrame` callbacks and compare it to a
 *         16.67 ms target (60 fps). A slowdown below 60 fps signals
 *         that either the audio thread or main-thread work is
 *         spending budget. This is an *estimate* — not the kernel's
 *         CPU% — but it tracks user-perceived performance closely
 *         enough to drive a single-bar toolbar indicator. Smoothed
 *         over a 16-sample ring buffer so the value doesn't jitter
 *         every frame.
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
  /** 0-100, smoothed frame-time deviation from the 60fps target. */
  cpuPct: number
  /** Absolute MB used by the JS heap, or null when not available. */
  memMb: number | null
  /** 0-1 ratio of usedJSHeapSize / totalJSHeapSize, or null. */
  memRatio: number | null
  set: (next: { cpuPct: number; memMb: number | null; memRatio: number | null }) => void
}

export const usePerfMetersStore = create<PerfMetersState>((set) => ({
  cpuPct: 0,
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

  const TARGET_FRAME_MS = 1000 / 60
  const BUF = 16
  const samples: number[] = []
  let lastTs = performance.now()
  let rafId = 0
  const tick = (ts: number) => {
    const dt = ts - lastTs
    lastTs = ts
    samples.push(dt)
    if (samples.length > BUF) samples.shift()
    rafId = requestAnimationFrame(tick)
  }
  rafId = requestAnimationFrame(tick)

  const intervalId = window.setInterval(() => {
    const avg = samples.length > 0
      ? samples.reduce((s, v) => s + v, 0) / samples.length
      : TARGET_FRAME_MS
    // Map a frame budget of TARGET → 0%, 2× TARGET → 50%, 4× TARGET → 100%.
    const dev = Math.max(0, avg - TARGET_FRAME_MS) / TARGET_FRAME_MS
    const cpuPct = Math.min(100, Math.round(dev * 50))

    // performance.memory is Chrome / Chromium / Tauri-webview only.
    let memMb: number | null = null
    let memRatio: number | null = null
    const pm = (performance as unknown as { memory?: { usedJSHeapSize: number; totalJSHeapSize: number } }).memory
    if (pm && typeof pm.usedJSHeapSize === 'number' && typeof pm.totalJSHeapSize === 'number') {
      memMb = Math.round(pm.usedJSHeapSize / (1024 * 1024))
      memRatio = pm.totalJSHeapSize > 0 ? pm.usedJSHeapSize / pm.totalJSHeapSize : null
    }

    usePerfMetersStore.getState().set({ cpuPct, memMb, memRatio })
  }, 200)

  cleanup = () => {
    cancelAnimationFrame(rafId)
    window.clearInterval(intervalId)
    cleanup = null
  }
  return cleanup
}
