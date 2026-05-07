/*
 * AutomationLane — paarse curve-strip rendered direct under the track
 * row inside the playlist. Phase 1 of the automation UI rollout:
 *  - reads `track.automationLanes` from the track store
 *  - draws the curve as an SVG path scaled to the lane's pixel width
 *  - dubbel-click op de lijn voegt een nieuw punt toe (mid-value)
 *  - drag op een punt = move (commit op mouseup)
 *  - delete-knop in de lane label verwijdert hem
 *
 * Stays narrow on purpose. Curve modes, target picker, snap, and
 * plugin-param target ship in follow-up commits per the mockup
 * contract at /var/www/hardwave-app/automation-lane-mockup/.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  type AutomationLaneInfo,
  type AutomationTargetInfo,
  useTrackStore,
} from '../stores/trackStore'
import { snapToTicks, useTransportStore } from '../stores/transportStore'

const PPQ = 960

type CurveMode = 'linear' | 'bezier' | 'step' | 'stairs' | 'smooth_stairs'
const CURVE_MODES: { id: CurveMode; label: string }[] = [
  { id: 'linear',        label: 'Linear' },
  { id: 'bezier',        label: 'Bezier' },
  { id: 'step',          label: 'Step' },
  { id: 'stairs',        label: 'Stairs' },
  { id: 'smooth_stairs', label: 'Smooth Stairs' },
]
function curveFromBackend(s: string): CurveMode {
  // Rust serializes via Debug ("Linear", "Bezier", ...); strip + lowercase.
  const k = s.toLowerCase().replace(/\s+/g, '_')
  return (CURVE_MODES.find(c => c.id === k)?.id) ?? 'linear'
}

interface Props {
  trackId: string
  lane: AutomationLaneInfo
}

export function AutomationLane({ trackId, lane }: Props) {
  const addPoint = useTrackStore(s => s.addAutomationPoint)
  const movePoint = useTrackStore(s => s.moveAutomationPoint)
  const deletePoint = useTrackStore(s => s.deleteAutomationPoint)
  const deleteLane = useTrackStore(s => s.deleteAutomationLane)
  const setVisible = useTrackStore(s => s.setAutomationLaneVisible)
  const horizontalZoom = useTransportStore(s => s.horizontalZoom)
  const trackHeight = useTransportStore(s => s.trackHeight)
  const snapValue = useTransportStore(s => s.snapValue)
  const snapEnabled = useTransportStore(s => s.snapEnabled)
  const bodyRef = useRef<HTMLDivElement | null>(null)
  const hostRef = useRef<HTMLDivElement | null>(null)
  // Measure the playlist grid width dynamically so the lane body
  // overlay extends across the full timeline. Re-measure on resize.
  const [bodyWidth, setBodyWidth] = useState<number>(1200)
  useEffect(() => {
    const update = () => {
      const grid = document.querySelector('.fl-pl-grid') as HTMLElement | null
      const tracksList = document.querySelector('.fl-pl-tracks-list') as HTMLElement | null
      if (!grid || !tracksList) return
      // Body width = playlist grid width. Label sits over the
      // tracks-list column so total horizontal coverage is the
      // playlist body width.
      setBodyWidth(grid.offsetWidth)
    }
    update()
    window.addEventListener('resize', update)
    return () => window.removeEventListener('resize', update)
  }, [horizontalZoom])

  // Drag state. We track which point is being dragged + its original
  // tick/value so we can render a live preview without round-tripping
  // through the store on every mousemove.
  const [drag, setDrag] = useState<
    | { pointIndex: number; tick: number; value: number }
    | null
  >(null)

  /** Right-click context menu state — anchored to the dot under the cursor. */
  const [ctx, setCtx] = useState<
    | { pointIndex: number; x: number; y: number }
    | null
  >(null)
  // Close the menu if user clicks anywhere else.
  useEffect(() => {
    if (!ctx) return
    const onAnyClick = () => setCtx(null)
    window.addEventListener('mousedown', onAnyClick)
    return () => window.removeEventListener('mousedown', onAnyClick)
  }, [ctx])

  const setCurve = useCallback(
    async (pointIndex: number, mode: CurveMode) => {
      // Use invoke directly — the store's helper would refetch tracks
      // before we close the menu, leading to a tiny visual jitter.
      await invoke('set_automation_point_curve', {
        trackId,
        laneId: lane.id,
        pointIndex,
        curve: mode,
      })
      // Refresh the lane snapshot after the curve change.
      await useTrackStore.getState().fetchTracks()
      setCtx(null)
    },
    [trackId, lane.id],
  )

  const targetLabel = describeTarget(lane.target)

  // Visible bars on the playlist — must match the playlist's own
  // horizontal mapping. The playlist uses 96px per bar at zoom 1.0.
  const PX_PER_BAR = 96 * horizontalZoom
  const TICKS_PER_BAR = PPQ * 4

  /** Convert a tick to an x pixel inside this lane's body. */
  const tickToX = useCallback(
    (tick: number) => (tick / TICKS_PER_BAR) * PX_PER_BAR,
    [PX_PER_BAR, TICKS_PER_BAR],
  )
  /** Convert a normalized value (0..1) to a y pixel. y=0 top means value=1. */
  const valueToY = useCallback(
    (value: number) => (1 - value) * trackHeight,
    [trackHeight],
  )
  /** Inverse: a click at (clientX, clientY) → (tick, value). Snaps the
   * tick to the playlist's current grid setting when snap is enabled,
   * so points always land on bar/beat boundaries you can predict. */
  const eventToTickValue = useCallback(
    (e: { clientX: number; clientY: number }): { tick: number; value: number } | null => {
      const el = bodyRef.current
      if (!el) return null
      const r = el.getBoundingClientRect()
      const x = Math.max(0, e.clientX - r.left)
      const y = Math.max(0, Math.min(r.height, e.clientY - r.top))
      let tick = Math.round((x / PX_PER_BAR) * TICKS_PER_BAR)
      const snap = snapToTicks(snapValue, snapEnabled)
      if (snap > 0) {
        tick = Math.round(tick / snap) * snap
      }
      const value = 1 - y / r.height
      return { tick, value: Math.max(0, Math.min(1, value)) }
    },
    [PX_PER_BAR, TICKS_PER_BAR, snapValue, snapEnabled],
  )

  // Build the SVG path connecting all points, treating the lane's
  // horizontal extent as the viewport. Empty / single-point lanes get
  // a flat line at the (only) value or the default 0.5.
  const pathD = useCallback(() => {
    if (lane.points.length === 0) return ''
    if (lane.points.length === 1) {
      const x = tickToX(lane.points[0].tick)
      const y = valueToY(lane.points[0].value)
      return `M0,${y} L${x},${y}`
    }
    let d = ''
    lane.points.forEach((p, i) => {
      const x = tickToX(p.tick)
      const y = valueToY(p.value)
      d += i === 0 ? `M${x},${y}` : ` L${x},${y}`
    })
    return d
  }, [lane.points, tickToX, valueToY])

  /** Double-click on empty lane area → insert a new point there. */
  const onLaneDoubleClick = useCallback(
    async (e: React.MouseEvent<HTMLDivElement>) => {
      // Bail if the click landed on a dot — its handler manages this.
      if ((e.target as HTMLElement).classList.contains('fl-lane-dot')) return
      const tv = eventToTickValue(e)
      if (!tv) return
      await addPoint(trackId, lane.id, tv.tick, tv.value)
    },
    [trackId, lane.id, addPoint, eventToTickValue],
  )

  /** Drag a single point. */
  const beginDrag = useCallback(
    (idx: number, e: React.MouseEvent) => {
      e.preventDefault()
      e.stopPropagation()
      setDrag({ pointIndex: idx, tick: lane.points[idx].tick, value: lane.points[idx].value })
    },
    [lane.points],
  )

  useEffect(() => {
    if (drag === null) return
    const onMove = (e: MouseEvent) => {
      const tv = eventToTickValue(e)
      if (!tv) return
      setDrag(prev => (prev ? { ...prev, tick: tv.tick, value: tv.value } : prev))
    }
    const onUp = async () => {
      // Commit the final tick/value. Drag may have crossed neighbouring
      // points, so the new index can differ from the original.
      if (drag) {
        await movePoint(trackId, lane.id, drag.pointIndex, drag.tick, drag.value)
      }
      setDrag(null)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
  }, [drag, trackId, lane.id, movePoint, eventToTickValue])

  return (
    <div
      ref={hostRef}
      className={`fl-lane${lane.visible ? '' : ' hidden'}`}
      style={{ height: trackHeight, ['--lane-body-w' as any]: `${bodyWidth}px` }}
      data-lane-id={lane.id}
    >
      <div className="fl-lane-label">
        <span className="led" />
        <span className="target">▾ {targetLabel}</span>
        <button
          type="button"
          className="vis"
          title={lane.visible ? 'Hide lane (bypass automation)' : 'Show lane (re-engage automation)'}
          onClick={() => setVisible(trackId, lane.id, !lane.visible)}
        >
          {lane.visible ? '◉' : '◌'}
        </button>
        <button
          type="button"
          className="del"
          title="Delete automation lane"
          onClick={() => deleteLane(trackId, lane.id)}
        >
          ×
        </button>
      </div>
      <div
        ref={bodyRef}
        className="fl-lane-body"
        onDoubleClick={onLaneDoubleClick}
      >
        <svg className="fl-lane-svg" preserveAspectRatio="none">
          <path d={pathD()} />
        </svg>
        {lane.points.map((p, i) => {
          const isDragging = drag?.pointIndex === i
          const tick = isDragging ? drag!.tick : p.tick
          const value = isDragging ? drag!.value : p.value
          const left = tickToX(tick)
          const top = valueToY(value)
          return (
            <div
              key={i}
              className={`fl-lane-dot${isDragging ? ' dragging' : ''}`}
              style={{ left, top }}
              onMouseDown={(e) => beginDrag(i, e)}
              onContextMenu={(e) => {
                e.preventDefault()
                e.stopPropagation()
                const r = bodyRef.current?.getBoundingClientRect()
                setCtx({
                  pointIndex: i,
                  x: e.clientX - (r?.left ?? 0),
                  y: e.clientY - (r?.top ?? 0),
                })
              }}
              onDoubleClick={(e) => {
                e.stopPropagation()
                deletePoint(trackId, lane.id, i)
              }}
              title={`tick ${tick} · value ${(value * 100).toFixed(0)}% · ${p.curve} · right-click for curve modes · double-click to delete`}
            />
          )
        })}
        {ctx && (
          <div
            className="fl-lane-ctx"
            style={{ left: ctx.x, top: ctx.y }}
            onMouseDown={(e) => e.stopPropagation()}
          >
            {CURVE_MODES.map(m => {
              const active = curveFromBackend(lane.points[ctx.pointIndex]?.curve ?? '') === m.id
              return (
                <div
                  key={m.id}
                  className={`item${active ? ' active' : ''}`}
                  onClick={() => setCurve(ctx.pointIndex, m.id)}
                >
                  <span>{m.label}</span>
                  <span className="check">{active ? '✓' : ''}</span>
                </div>
              )
            })}
            <div className="sep" />
            <div
              className="item danger"
              onClick={() => {
                deletePoint(trackId, lane.id, ctx.pointIndex)
                setCtx(null)
              }}
            >
              <span>Delete point</span>
              <span className="kbd">Del</span>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

function describeTarget(t: AutomationTargetInfo): string {
  switch (t.kind) {
    case 'track_volume': return 'VOLUME'
    case 'track_pan':    return 'PAN'
    case 'track_mute':   return 'MUTE'
    case 'plugin_param': return `PLUGIN ${t.paramId}`
    case 'send_level':   return `SEND ${t.sendIndex}`
  }
}
