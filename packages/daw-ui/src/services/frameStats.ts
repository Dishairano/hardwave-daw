/**
 * What the FPS meter shows: how smoothly the interface draws, and what is
 * keeping it busy. Frame times come from requestAnimationFrame, long tasks
 * (the main thread blocked for 50 ms or more) from the browser, renders
 * from React's commit hook, and engine events from the stores that receive
 * them. Sampling only runs while the meter is on.
 */

let backendEvents = 0

/** One message from the audio engine (transport, meters) reached the page. */
export function noteBackendEvent(): void {
  backendEvents++
}

/** React commits so far, counted by public/render-counter.js, which runs
 *  before the app so the hook is there when react-dom loads. */
function commitCount(): number {
  return (window as unknown as { __hwCommits?: number }).__hwCommits ?? 0
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
  let commitsAt = commitCount()
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
        rendersPerSec: (commitCount() - commitsAt) / secs,
        engineEventsPerSec: (backendEvents - eventsAt) / secs,
        heapMb: memory ? memory.usedJSHeapSize / (1024 * 1024) : null,
        recent: recent.slice(),
      })
      frames = []
      windowStart = now
      longCount = 0
      longMs = 0
      commitsAt = commitCount()
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
