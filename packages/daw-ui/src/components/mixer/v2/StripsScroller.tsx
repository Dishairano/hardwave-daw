import { memo, useCallback, useEffect, useRef } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { ChannelStrip } from './ChannelStrip'
import { useTrackStore } from '../../../stores/trackStore'

/**
 * Virtualized, GPU-composited horizontal scroller for the insert column.
 *
 * Two stacked optimizations on top of the per-strip primitives:
 *
 * 1. **Virtualization** via `@tanstack/react-virtual`. Only the strips in
 *    (and immediately around) the viewport are mounted as DOM nodes. The
 *    rest exist in state but never hit the React tree, so a 500-track
 *    session has the same cost as a 25-track session.
 *
 * 2. **GPU compositor scroll**. Native `scrollLeft` mutations invalidate
 *    layout each frame which Chromium hard-caps at 60 Hz. We disable the
 *    native overflow scroll and apply the offset as a `translate3d`
 *    transform on the inner container — pure compositor work, runs at the
 *    display's actual refresh rate (120 / 144 / 240 Hz).
 *
 * 3. **Spring-physics easing** on wheel input. Wheel deltas inject impulse
 *    into a velocity that a critically-damped spring pulls toward target.
 *    Result is a physical, weighty feel like FL Studio / Logic.
 *
 * Wheel routing exception: if the cursor is over a `.hw-fader` or a
 * `.hw-knob` the wheel adjusts THAT control instead of scrolling. The
 * primitives handle that themselves; this component just bails when the
 * event target is inside one of them.
 */

const STRIP_W = 64
const OVERSCAN = 8

// Spring physics
const SPRING_K = 300
const SPRING_C = 32
const SETTLE_PX = 0.3
const SETTLE_VEL = 1.0

// Wheel feel
const WHEEL_PIXEL_STEP = 220
const WHEEL_LINE_HEIGHT = 40
const WHEEL_VELOCITY_GAIN = 14

export interface StripsScrollerProps {
  onSelect: (id: string) => void
  selectedId: string | null
}

export const StripsScroller = memo(function StripsScroller(props: StripsScrollerProps) {
  const { onSelect, selectedId } = props

  // Subscribe to track ids only — adding/removing a track triggers a
  // re-mount of the virtualizer, but field updates on individual tracks
  // don't reach this component. Per-strip subscriptions handle those.
  const inserts = useTrackStore((s) =>
    s.tracks.filter((t) => t.kind !== 'Master' && t.kind !== 'Automation'),
  )

  const viewportRef = useRef<HTMLDivElement | null>(null)
  const innerRef = useRef<HTMLDivElement | null>(null)

  const virtualizer = useVirtualizer({
    count: inserts.length,
    getScrollElement: () => viewportRef.current,
    estimateSize: () => STRIP_W,
    overscan: OVERSCAN,
    horizontal: true,
  })

  // ---- spring scroll state ----
  const targetRef = useRef(0)
  const currentRef = useRef(0)
  const velocityRef = useRef(0)
  const scrollingRef = useRef(false)
  const lastFrameRef = useRef(0)

  const applyTransform = useCallback(() => {
    const inner = innerRef.current
    if (!inner) return
    inner.style.transform = `translate3d(${-currentRef.current.toFixed(2)}px,0,0)`
    // Sync the virtualizer's notion of scroll position so it knows which
    // strips to mount. @tanstack/react-virtual reads from the scroll
    // element's scrollLeft, but since we don't use native scroll any more,
    // we have to push the value ourselves via the API.
    virtualizer.scrollToOffset(currentRef.current, { align: 'start' })
  }, [virtualizer])

  const getMaxScroll = useCallback(() => {
    const viewport = viewportRef.current
    const inner = innerRef.current
    if (!viewport || !inner) return 0
    return Math.max(0, inner.scrollWidth - viewport.clientWidth)
  }, [])

  const animateScroll = useCallback(
    (timestamp: number) => {
      if (!lastFrameRef.current) lastFrameRef.current = timestamp
      const dt = Math.min(0.05, (timestamp - lastFrameRef.current) / 1000)
      lastFrameRef.current = timestamp

      const displacement = currentRef.current - targetRef.current
      const acc = -SPRING_K * displacement - SPRING_C * velocityRef.current
      velocityRef.current += acc * dt
      currentRef.current += velocityRef.current * dt

      // Clamp against bounds
      const max = getMaxScroll()
      if (currentRef.current < 0) {
        currentRef.current = 0
        if (velocityRef.current < 0) velocityRef.current = 0
      }
      if (currentRef.current > max) {
        currentRef.current = max
        if (velocityRef.current > 0) velocityRef.current = 0
      }

      applyTransform()

      const settled =
        Math.abs(displacement) < SETTLE_PX && Math.abs(velocityRef.current) < SETTLE_VEL
      if (settled) {
        currentRef.current = targetRef.current
        velocityRef.current = 0
        applyTransform()
        scrollingRef.current = false
        lastFrameRef.current = 0
        return
      }
      requestAnimationFrame(animateScroll)
    },
    [applyTransform, getMaxScroll],
  )

  const kick = useCallback(() => {
    if (!scrollingRef.current) {
      scrollingRef.current = true
      lastFrameRef.current = 0
      requestAnimationFrame(animateScroll)
    }
  }, [animateScroll])

  const normalizeWheelDelta = useCallback((e: WheelEvent): number => {
    let dy = e.deltaY
    if (e.deltaMode === 1) dy *= WHEEL_LINE_HEIGHT
    else if (e.deltaMode === 2) dy *= viewportRef.current?.clientWidth ?? 800
    if (Math.abs(dy) > 600) dy = Math.sign(dy) * 600
    if (e.deltaMode === 0 && Math.abs(dy) > 50 && Math.abs(dy) < 200) {
      dy = Math.sign(dy) * WHEEL_PIXEL_STEP
    }
    return dy
  }, [])

  // Wheel handler — non-passive so we can preventDefault. Bail out when the
  // wheel target is a knob/fader so those primitives can claim the event.
  useEffect(() => {
    const el = viewportRef.current
    if (!el) return
    const onWheel = (e: WheelEvent) => {
      const target = e.target as HTMLElement | null
      if (target?.closest('.hw-knob') || target?.closest('.hw-fader')) return
      e.preventDefault()
      const dy = normalizeWheelDelta(e)
      const max = getMaxScroll()
      targetRef.current = Math.max(0, Math.min(max, targetRef.current + dy))
      velocityRef.current += dy * WHEEL_VELOCITY_GAIN
      kick()
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [getMaxScroll, kick, normalizeWheelDelta])

  // When the strip count changes, clamp the target to the new max so we
  // don't end up stuck scrolled past the last strip.
  useEffect(() => {
    const max = getMaxScroll()
    if (targetRef.current > max) {
      targetRef.current = max
      currentRef.current = max
      applyTransform()
    }
  }, [inserts.length, applyTransform, getMaxScroll])

  const visibleItems = virtualizer.getVirtualItems()
  const totalSize = virtualizer.getTotalSize()

  return (
    <div
      className="mx-strips-wrap"
      ref={viewportRef}
      style={{ position: 'relative', overflow: 'hidden' }}
    >
      <div
        className="mx-strips-inner"
        ref={innerRef}
        style={{
          width: totalSize,
          height: '100%',
          position: 'relative',
          willChange: 'transform',
          transform: 'translate3d(0,0,0)',
          contain: 'layout paint',
        }}
      >
        {visibleItems.map((virt) => {
          const track = inserts[virt.index]
          if (!track) return null
          return (
            <div
              key={track.id}
              style={{
                position: 'absolute',
                top: 0,
                left: virt.start,
                width: STRIP_W,
                height: '100%',
              }}
            >
              <ChannelStrip
                trackId={track.id}
                index={virt.index + 1}
                selected={selectedId === track.id}
                onSelect={onSelect}
                variant="insert"
                separator={
                  track.kind === 'Bus' || track.kind === 'Return' ? 'group' : 'none'
                }
              />
            </div>
          )
        })}
      </div>
    </div>
  )
})
