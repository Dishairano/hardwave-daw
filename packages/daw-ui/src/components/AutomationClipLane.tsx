/*
 * AutomationClipLane — renders one arrangement-level automation clip as a
 * curve strip positioned at its [startTick, startTick+lengthTicks] window
 * on the playlist timeline. Points are stored clip-local (0..lengthTicks);
 * we offset them by startTick for drawing and convert back on edit.
 *
 *  - double-click inside the clip window → add a point
 *  - drag a point → move (commit on mouseup)
 *  - right-click a point → delete
 *  - × in the label → delete the whole clip
 *
 * Mirrors AutomationLane's coordinate math so it lines up pixel-for-pixel
 * with the playlist grid.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { type AutomationClipInfo, type AutomationTargetInfo, useTrackStore } from '../stores/trackStore'
import { snapToTicks, useTransportStore } from '../stores/transportStore'

const PPQ = 960

function describeTarget(t: AutomationTargetInfo): string {
  switch (t.kind) {
    case 'track_volume': return 'Volume'
    case 'track_pan': return 'Pan'
    case 'track_mute': return 'Mute'
    case 'plugin_param': return `Plugin p${t.paramId}`
    case 'send_level': return `Send ${t.sendIndex + 1}`
    default: return 'Automation'
  }
}

interface Props {
  trackId: string
  clip: AutomationClipInfo
}

export function AutomationClipLane({ trackId, clip }: Props) {
  const moveClip = useTrackStore(s => s.moveAutomationClip)
  const deleteClip = useTrackStore(s => s.deleteAutomationClip)
  const addPoint = useTrackStore(s => s.addAutomationClipPoint)
  const movePoint = useTrackStore(s => s.moveAutomationClipPoint)
  const removePoint = useTrackStore(s => s.removeAutomationClipPoint)
  const horizontalZoom = useTransportStore(s => s.horizontalZoom)
  const trackHeight = useTransportStore(s => s.trackHeight)
  const snapValue = useTransportStore(s => s.snapValue)
  const snapEnabled = useTransportStore(s => s.snapEnabled)
  const bodyRef = useRef<HTMLDivElement | null>(null)

  const [drag, setDrag] = useState<{ pointIndex: number; tick: number; value: number } | null>(null)

  const PX_PER_BAR = 96 * horizontalZoom
  const TICKS_PER_BAR = PPQ * 4
  const tickToX = useCallback((tick: number) => (tick / TICKS_PER_BAR) * PX_PER_BAR, [PX_PER_BAR, TICKS_PER_BAR])
  const valueToY = useCallback((value: number) => (1 - value) * trackHeight, [trackHeight])

  // A click → clip-local tick (snapped) + value. Returns null outside the body.
  const eventToLocal = useCallback(
    (e: { clientX: number; clientY: number }): { tick: number; value: number } | null => {
      const el = bodyRef.current
      if (!el) return null
      const r = el.getBoundingClientRect()
      const x = Math.max(0, e.clientX - r.left)
      const y = Math.max(0, Math.min(r.height, e.clientY - r.top))
      let timelineTick = Math.round((x / PX_PER_BAR) * TICKS_PER_BAR)
      const snap = snapToTicks(snapValue, snapEnabled)
      if (snap > 0) timelineTick = Math.round(timelineTick / snap) * snap
      // Clamp to the clip's window, then make it clip-local.
      const local = Math.max(0, Math.min(clip.lengthTicks, timelineTick - clip.startTick))
      return { tick: local, value: Math.max(0, Math.min(1, 1 - y / r.height)) }
    },
    [PX_PER_BAR, TICKS_PER_BAR, snapValue, snapEnabled, clip.startTick, clip.lengthTicks],
  )

  const pathD = useCallback(() => {
    if (clip.points.length === 0) return ''
    const x0 = tickToX(clip.startTick)
    if (clip.points.length === 1) {
      const y = valueToY(clip.points[0].value)
      return `M${x0},${y} L${tickToX(clip.startTick + clip.points[0].tick)},${y}`
    }
    let d = ''
    clip.points.forEach((p, i) => {
      const x = tickToX(clip.startTick + p.tick)
      const y = valueToY(p.value)
      d += i === 0 ? `M${x},${y}` : ` L${x},${y}`
    })
    return d
  }, [clip.points, clip.startTick, tickToX, valueToY])

  const onBodyDoubleClick = useCallback(
    async (e: React.MouseEvent<HTMLDivElement>) => {
      if ((e.target as HTMLElement).classList.contains('fl-lane-dot')) return
      const tv = eventToLocal(e)
      if (!tv) return
      await addPoint(trackId, clip.id, tv.tick, tv.value)
    },
    [trackId, clip.id, addPoint, eventToLocal],
  )

  const beginDrag = useCallback((idx: number, e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    const p = clip.points[idx]
    setDrag({ pointIndex: idx, tick: p.tick, value: p.value })
  }, [clip.points])

  useEffect(() => {
    if (!drag) return
    const onMove = (e: MouseEvent) => {
      const tv = eventToLocal(e)
      if (tv) setDrag(d => (d ? { ...d, tick: tv.tick, value: tv.value } : d))
    }
    const onUp = async () => {
      const d = drag
      setDrag(null)
      if (d) await movePoint(trackId, clip.id, d.pointIndex, d.tick, d.value)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
  }, [drag, trackId, clip.id, movePoint, eventToLocal])

  const clipColor = `#${(clip.colorArgb & 0xffffff).toString(16).padStart(6, '0')}`
  const clipLeft = tickToX(clip.startTick)
  const clipWidth = tickToX(clip.lengthTicks)

  return (
    <div className="fl-lane" style={{ height: trackHeight }} data-automation-clip-id={clip.id}>
      <div className="fl-lane-label">
        <span className="led" style={{ background: clipColor }} />
        <span className="target">◆ {describeTarget(clip.target)}</span>
        <button type="button" className="del" title="Delete automation clip" onClick={() => deleteClip(trackId, clip.id)}>
          ×
        </button>
      </div>
      <div ref={bodyRef} className="fl-lane-body" onDoubleClick={onBodyDoubleClick}>
        {/* Clip window tint */}
        <div
          title="Drag header to move clip"
          onMouseDown={(e) => {
            // Drag the whole clip when grabbing its tinted body (not a dot).
            if ((e.target as HTMLElement).classList.contains('fl-lane-dot')) return
            const startX = e.clientX
            const origStart = clip.startTick
            const onMove = (ev: MouseEvent) => {
              const dxTicks = Math.round(((ev.clientX - startX) / (96 * horizontalZoom)) * TICKS_PER_BAR)
              ;(e.target as HTMLElement).dataset.pending = String(Math.max(0, origStart + dxTicks))
            }
            const onUp = (ev: MouseEvent) => {
              window.removeEventListener('mousemove', onMove)
              window.removeEventListener('mouseup', onUp)
              const dxTicks = Math.round(((ev.clientX - startX) / (96 * horizontalZoom)) * TICKS_PER_BAR)
              if (dxTicks !== 0) moveClip(trackId, clip.id, Math.max(0, origStart + dxTicks))
            }
            window.addEventListener('mousemove', onMove)
            window.addEventListener('mouseup', onUp)
          }}
          style={{
            position: 'absolute', left: clipLeft, width: Math.max(2, clipWidth), top: 0, bottom: 0,
            background: `${clipColor}1a`, borderLeft: `2px solid ${clipColor}`, cursor: 'grab',
          }}
        />
        <svg className="fl-lane-svg" preserveAspectRatio="none">
          <path d={pathD()} stroke={clipColor} fill="none" />
        </svg>
        {clip.points.map((p, i) => {
          const isDragging = drag?.pointIndex === i
          const tick = isDragging ? drag!.tick : p.tick
          const value = isDragging ? drag!.value : p.value
          return (
            <div
              key={i}
              className={`fl-lane-dot${isDragging ? ' dragging' : ''}`}
              style={{ left: tickToX(clip.startTick + tick), top: valueToY(value) }}
              onMouseDown={(e) => beginDrag(i, e)}
              onContextMenu={(e) => { e.preventDefault(); removePoint(trackId, clip.id, i) }}
            />
          )
        })}
      </div>
    </div>
  )
}
