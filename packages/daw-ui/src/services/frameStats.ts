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

// Calls to the app's backend, timed while the meter runs: a window can draw
// at 60 FPS and still lag when the answers it waits for come late.
let ipcOn = false
let ipcCalls = 0
let ipcTotalMs = 0
let ipcMaxMs = 0
let ipcSlowest = ''

/** Time every backend call from now on. Wraps the function @tauri-apps/api
 *  looks up on each call, so it works whenever it is turned on. */
function timeBackendCalls(): void {
  type Internals = { invoke?: (cmd: string, ...rest: unknown[]) => Promise<unknown>; __hwTimed?: boolean }
  const internals = (window as unknown as { __TAURI_INTERNALS__?: Internals }).__TAURI_INTERNALS__
  if (!internals?.invoke || internals.__hwTimed) return
  const original = internals.invoke.bind(internals)
  internals.invoke = (cmd: string, ...rest: unknown[]) => {
    if (!ipcOn || cmd.startsWith('plugin:event|')) return original(cmd, ...rest)
    const t0 = performance.now()
    const done = () => {
      const ms = performance.now() - t0
      ipcCalls++
      ipcTotalMs += ms
      if (ms > ipcMaxMs) { ipcMaxMs = ms; ipcSlowest = cmd }
    }
    const p = original(cmd, ...rest)
    p.then(done, done)
    return p
  }
  internals.__hwTimed = true
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
  /** Backend calls a second, their average and longest round trip (ms),
   *  and which command took longest. */
  ipcPerSec: number
  ipcAvgMs: number
  ipcMaxMs: number
  ipcSlowest: string
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
  timeBackendCalls()
  ipcOn = true
  ipcCalls = 0; ipcTotalMs = 0; ipcMaxMs = 0; ipcSlowest = ''

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
        ipcPerSec: ipcCalls / secs,
        ipcAvgMs: ipcCalls ? ipcTotalMs / ipcCalls : 0,
        ipcMaxMs,
        ipcSlowest,
        heapMb: memory ? memory.usedJSHeapSize / (1024 * 1024) : null,
        recent: recent.slice(),
      })
      frames = []
      windowStart = now
      longCount = 0
      longMs = 0
      commitsAt = commitCount()
      eventsAt = backendEvents
      ipcCalls = 0; ipcTotalMs = 0; ipcMaxMs = 0; ipcSlowest = ''
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
    ipcOn = false
  }
}
