import { useEffect, useRef } from 'react'

/**
 * Multi-touch gesture detector for canvas / scroll containers.
 *
 * Wires native TouchEvent listeners onto the supplied element ref and
 * fires high-level callbacks for the three gestures the FL Studio
 * manual page "The User Interface" describes:
 *
 *  - **Pinch zoom** — two fingers spread/pinch → `onPinch(factor, cx, cy)`.
 *    Factor is `currentDistance / startDistance` so >1 = spread,
 *    <1 = pinch. Center is the midpoint between the two fingers.
 *  - **Two-finger pan** — two fingers drag together → `onPan(dx, dy)`.
 *    Deltas are screen pixels relative to the previous event.
 *  - **Double tap** — two quick taps at roughly the same location →
 *    `onDoubleTap(x, y)`.
 *
 * Single-finger drag is left alone — the caller's existing pointerdown
 * / mousemove handlers continue to handle that. Browser default scroll
 * + pinch-zoom on the target are suppressed via `touch-action: none`
 * (callers set this on the element CSS, the hook calls
 * `preventDefault` on every touch event too as belt-and-braces).
 */

export interface MultiTouchCallbacks {
  /** Called continuously while two fingers pinch/spread. */
  onPinch?: (factor: number, centerX: number, centerY: number) => void
  /** Called continuously while two fingers drag together. */
  onPan?: (dx: number, dy: number) => void
  /** Called once on a recognised double-tap. */
  onDoubleTap?: (x: number, y: number) => void
}

const DOUBLE_TAP_MS = 300
const DOUBLE_TAP_RADIUS_PX = 28

interface TouchState {
  startDistance: number
  startCenterX: number
  startCenterY: number
  lastCenterX: number
  lastCenterY: number
  lastTapAt: number
  lastTapX: number
  lastTapY: number
}

function distance(a: Touch, b: Touch): number {
  const dx = a.clientX - b.clientX
  const dy = a.clientY - b.clientY
  return Math.sqrt(dx * dx + dy * dy)
}

function center(a: Touch, b: Touch): { x: number; y: number } {
  return { x: (a.clientX + b.clientX) / 2, y: (a.clientY + b.clientY) / 2 }
}

export function useMultiTouchGestures(
  ref: React.RefObject<HTMLElement | null>,
  callbacks: MultiTouchCallbacks,
): void {
  // Hold the latest callbacks in a ref so we don't re-attach listeners
  // on every render. Caller can pass inline lambdas without thrashing.
  const cbRef = useRef(callbacks)
  cbRef.current = callbacks

  useEffect(() => {
    const el = ref.current
    if (!el) return

    const state: TouchState = {
      startDistance: 0,
      startCenterX: 0,
      startCenterY: 0,
      lastCenterX: 0,
      lastCenterY: 0,
      lastTapAt: 0,
      lastTapX: 0,
      lastTapY: 0,
    }

    const onStart = (e: TouchEvent) => {
      if (e.touches.length === 2) {
        e.preventDefault()
        const [a, b] = [e.touches[0], e.touches[1]]
        state.startDistance = distance(a, b)
        const c = center(a, b)
        state.startCenterX = c.x
        state.startCenterY = c.y
        state.lastCenterX = c.x
        state.lastCenterY = c.y
      } else if (e.touches.length === 1) {
        // Detect double-tap by remembering the previous touchend's
        // time + position and comparing on the next touchstart.
        const now = performance.now()
        const t = e.touches[0]
        const dx = t.clientX - state.lastTapX
        const dy = t.clientY - state.lastTapY
        if (
          now - state.lastTapAt < DOUBLE_TAP_MS &&
          Math.sqrt(dx * dx + dy * dy) < DOUBLE_TAP_RADIUS_PX
        ) {
          e.preventDefault()
          cbRef.current.onDoubleTap?.(t.clientX, t.clientY)
          // Reset so triple-tap doesn't fire a second double-tap.
          state.lastTapAt = 0
        }
      }
    }

    const onMove = (e: TouchEvent) => {
      if (e.touches.length !== 2) return
      e.preventDefault()
      const [a, b] = [e.touches[0], e.touches[1]]
      const d = distance(a, b)
      const c = center(a, b)
      // Pinch (zoom). Factor is current / start, so >1 = spread.
      if (state.startDistance > 0) {
        const factor = d / state.startDistance
        if (Math.abs(factor - 1) > 0.005) {
          cbRef.current.onPinch?.(factor, c.x, c.y)
        }
      }
      // Pan (two-finger drag). Delta relative to the previous frame
      // rather than the start so the caller can integrate directly
      // into an offset without re-applying the initial position.
      const dx = c.x - state.lastCenterX
      const dy = c.y - state.lastCenterY
      if (dx !== 0 || dy !== 0) {
        cbRef.current.onPan?.(dx, dy)
      }
      state.lastCenterX = c.x
      state.lastCenterY = c.y
    }

    const onEnd = (e: TouchEvent) => {
      if (e.touches.length === 0 && e.changedTouches.length === 1) {
        // Single tap released. Stash for potential double-tap detection
        // on the NEXT touchstart.
        const t = e.changedTouches[0]
        state.lastTapAt = performance.now()
        state.lastTapX = t.clientX
        state.lastTapY = t.clientY
      }
      if (e.touches.length < 2) {
        // Two-finger gesture ended — reset pinch baseline so a follow-up
        // pinch doesn't compare against an old distance.
        state.startDistance = 0
      }
    }

    el.addEventListener('touchstart', onStart, { passive: false })
    el.addEventListener('touchmove', onMove, { passive: false })
    el.addEventListener('touchend', onEnd, { passive: false })
    el.addEventListener('touchcancel', onEnd, { passive: false })
    return () => {
      el.removeEventListener('touchstart', onStart)
      el.removeEventListener('touchmove', onMove)
      el.removeEventListener('touchend', onEnd)
      el.removeEventListener('touchcancel', onEnd)
    }
  }, [ref])
}
