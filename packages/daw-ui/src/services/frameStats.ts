/**
 * What the FPS meter shows: how smoothly the interface draws, and what is
 * keeping it busy. Frame times come from requestAnimationFrame, long tasks
 * (the main thread blocked for 50 ms or more) from the browser, renders
 * from React's commit hook, and engine events from the stores that receive
 * them. Sampling only runs while the meter is on.
 */

let commits = 0
let backendEvents = 0

/** One message from the audio engine (transport, meters) reached the page. */
export function noteBackendEvent(): void {
  backendEvents++
}

/**
 * Count React commits, through the hook React looks for when it loads.
 * Must run before react-dom is imported. When a hook is there already
 * (React Refresh in development) its commit callback is kept and wrapped.
 * Nothing of React's internals is kept: only the count.
 */
export function installCommitCounter(): void {
  if (typeof window === 'undefined') return
  type Hook = { onCommitFiberRoot?: (...args: unknown[]) => void } & Record<string, unknown>
  const w = window as unknown as { __REACT_DEVTOOLS_GLOBAL_HOOK__?: Hook }
  const hook = w.__REACT_DEVTOOLS_GLOBAL_HOOK__
  if (hook) {
    const previous = hook.onCommitFiberRoot
    hook.onCommitFiberRoot = (...args: unknown[]) => {
      commits++
      previous?.(...args)
    }
    return
  }
  w.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
    supportsFiber: true,
    isDisabled: false,
    renderers: new Map(),
    inject: () => 1,
    onCommitFiberRoot: () => { commits++ },
    onCommitFiberUnmount: () => {},
    onPostCommitFiberRoot: () => {},
    checkDCE: () => {},
  }
}

export interface FrameSnapshot {
  fps: number
  /** Average and longest frame in the window, ms. */
  avgMs: number
  worstMs: number
  /** Frames that took longer than two at 60 Hz (33 ms), per second. */
  slowPerSec: number
  /** Main-thread blocks of 50 ms or more, per second, and their total ms. */
  longTasksPerSec: number
  longTaskMs: number
  rendersPerSec: number
  engineEventsPerSec: number
  /** JavaScript heap in use, MB, where the browser reports it. */
  heapMb: number | null
  /** The last frame times, oldest first, for the graph. */
  recent: number[]
}

const WINDOW_MS = 500
const RECENT = 120

/**
 * Measure until the returned function is called. `onSnapshot` gets the
 * numbers twice a second.
 */
export function startFrameStats(onSnapshot: (s: FrameSnapshot) => void): () => void {
  const recent: number[] = []
  let frames: number[] = []
  let last = performance.now()
  let windowStart = last
  let raf = 0
  let longCount = 0
  let longMs = 0
  let commitsAt = commits
  let eventsAt = backendEvents

  let observer: PerformanceObserver | null = null
  try {
    observer = new PerformanceObserver((list) => {
      for (const e of list.getEntries()) {
        longCount++
        longMs += e.duration
      }
    })
    observer.observe({ entryTypes: ['longtask'] })
  } catch {
    observer = null
  }

  const frame = (now: number) => {
    const dt = now - last
    last = now
    frames.push(dt)
    recent.push(dt)
    if (recent.length > RECENT) recent.shift()
    const span = now - windowStart
    if (span >= WINDOW_MS) {
      const secs = span / 1000
      const total = frames.reduce((a, b) => a + b, 0)
      const memory = (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory
      onSnapshot({
        fps: frames.length / secs,
        avgMs: frames.length ? total / frames.length : 0,
        worstMs: frames.length ? Math.max(...frames) : 0,
        slowPerSec: frames.filter((f) => f > 33.4).length / secs,
        longTasksPerSec: longCount / secs,
        longTaskMs: longMs,
        rendersPerSec: (commits - commitsAt) / secs,
        engineEventsPerSec: (backendEvents - eventsAt) / secs,
        heapMb: memory ? memory.usedJSHeapSize / (1024 * 1024) : null,
        recent: recent.slice(),
      })
      frames = []
      windowStart = now
      longCount = 0
      longMs = 0
      commitsAt = commits
      eventsAt = backendEvents
    }
    raf = requestAnimationFrame(frame)
  }
  raf = requestAnimationFrame((now) => {
    last = now
    windowStart = now
    raf = requestAnimationFrame(frame)
  })

  return () => {
    cancelAnimationFrame(raf)
    observer?.disconnect()
  }
}
