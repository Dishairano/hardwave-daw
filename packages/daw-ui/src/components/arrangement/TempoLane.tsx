/**
 * The tempo lane's gestures, over the band the playlist's canvas draws it in
 * (see `tempoLane.ts`). As the mockup has it:
 *
 *   - right-click the lane to add a tempo change at the snap, or to set how a
 *     point leads to the next one (a ramp or a jump), type a tempo, delete a
 *     change or hide the lane;
 *   - drag a point up or down to set its tempo, sideways to move it, with
 *     the value shown while dragging;
 *   - double-click a point to type a value.
 *
 * A drag is one undo step: the engine hears each move (so the song follows
 * while it is played) inside one history group, named when it ends.
 */
import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useTempoMapStore, tempoChanged } from '../../stores/tempoMapStore'
import { useHistoryStore } from '../../stores/historyStore'
import { useGeneralPrefsStore } from '../../stores/generalPrefsStore'
import type { MeterSegment } from '../../utils/meter'
import { TEMPO_COLOR, TEMPO_LANE_HEIGHT, barLabel, bpmToY, changeAt, clampTick, laneScale, pointAt, roundBpm, yToBpm, type LaneScale } from './tempoLane'

interface Props {
  top: number
  pixelsPerTick: number
  getScrollOffset: () => number
  snapTicks: number
  segments: MeterSegment[]
  onWheel: (e: React.WheelEvent) => void
  onSetHint?: (hint: string) => void
}

interface Drag {
  index: number
  startX: number
  startY: number
  tick: number
  bpm: number
  ramp: string
  scale: LaneScale
  since: number
  moved: boolean
}

const fmt = (bpm: number) => `${bpm.toFixed(1)} BPM`

export function TempoLane({ top, pixelsPerTick, getScrollOffset, snapTicks, segments, onWheel, onSetHint }: Props) {
  const entries = useTempoMapStore((s) => s.entries)
  const frozen = useTempoMapStore((s) => s.laneScale)
  const ref = useRef<HTMLDivElement>(null)
  const drag = useRef<Drag | null>(null)
  const [dragging, setDragging] = useState(false)
  const [callout, setCallout] = useState<{ x: number; y: number; text: string } | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; tick: number; index: number; onPoint: boolean } | null>(null)
  const [typing, setTyping] = useState<{ x: number; y: number; index: number; draft: string } | null>(null)
  const [hover, setHover] = useState(false)

  const scale = frozen ?? laneScale(entries)
  const tickToX = (tick: number) => tick * pixelsPerTick - getScrollOffset()
  const xToTick = (x: number) => Math.max(0, (x + getScrollOffset()) / Math.max(1e-9, pixelsPerTick))
  const snap = (tick: number, free: boolean) =>
    !free && snapTicks > 0 ? Math.round(tick / snapTicks) * snapTicks : Math.round(tick)
  const local = (e: { clientX: number; clientY: number }) => {
    const r = ref.current?.getBoundingClientRect()
    return { x: e.clientX - (r?.left ?? 0), y: e.clientY - (r?.top ?? 0) }
  }

  // Each move is sent, a few times a second at most and always in order;
  // the last one waits for nothing.
  const send = useRef<{ chain: Promise<unknown>; last: number; timer: number | null; pending: Record<string, unknown> | null }>({
    chain: Promise.resolve(), last: 0, timer: null, pending: null,
  })
  const flush = () => {
    const q = send.current
    if (q.timer !== null) { window.clearTimeout(q.timer); q.timer = null }
    const args = q.pending
    q.pending = null
    if (!args) return q.chain
    q.last = Date.now()
    q.chain = q.chain.then(() => invoke('set_tempo_entry', args)).catch(() => {})
    return q.chain
  }
  const queue = (args: Record<string, unknown>) => {
    const q = send.current
    q.pending = args
    if (q.timer !== null) return
    q.timer = window.setTimeout(() => { q.timer = null; void flush() }, Math.max(0, 60 - (Date.now() - q.last)))
  }

  const calloutFor = (index: number, tick: number, bpm: number) => {
    const prev = entries[index - 1]
    const from = prev && prev.ramp === 'linear' ? ` · ramp from ${barLabel(segments, prev.tick)}` : ''
    const name = barLabel(segments, tick)
    return `${name[0].toUpperCase()}${name.slice(1)} · ${fmt(bpm)}${from}`
  }

  const onMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return
    setMenu(null)
    const { x, y } = local(e)
    const index = pointAt(entries, x, y, tickToX, scale)
    if (index === null) return
    e.preventDefault()
    const e0 = entries[index]
    drag.current = {
      index, startX: x, startY: y, tick: e0.tick, bpm: e0.bpm, ramp: e0.ramp,
      scale, since: Date.now(), moved: false,
    }
    // The scale holds still for the drag, or the point would run away from
    // the pointer as the song's range grew.
    useTempoMapStore.getState().setLaneScale(scale)
    useTempoMapStore.getState().setLaneHot(index)
    setDragging(true)
  }

  useEffect(() => {
    if (!dragging) return
    const move = (ev: MouseEvent) => {
      const d = drag.current
      if (!d) return
      const { x, y } = local(ev)
      if (!d.moved && Math.abs(x - d.startX) < 3 && Math.abs(y - d.startY) < 3) return
      if (!d.moved) {
        d.moved = true
        d.since = Date.now()
        // One gesture, one undo step.
        send.current.chain = send.current.chain.then(() => invoke('begin_history_group')).catch(() => {})
      }
      const bpm = roundBpm(yToBpm(bpmToY(d.bpm, d.scale) + (y - d.startY), d.scale))
      const wanted = d.tick + (x - d.startX) / Math.max(1e-9, pixelsPerTick)
      const tick = d.index === 0 ? 0 : clampTick(useTempoMapStore.getState().entries, d.index, snap(wanted, ev.altKey))
      useTempoMapStore.getState().previewEntry(d.index, tick, bpm)
      queue({ index: d.index, tick, bpm, ramp: d.ramp })
      setCallout({ x: tickToX(tick), y: bpmToY(bpm, d.scale), text: calloutFor(d.index, tick, bpm) })
    }
    const up = async () => {
      const d = drag.current
      drag.current = null
      setDragging(false)
      setCallout(null)
      const store = useTempoMapStore.getState()
      store.setLaneHot(null)
      if (!d || !d.moved) { store.setLaneScale(null); return }
      const now = store.entries[d.index]
      await flush()
      await invoke('end_history_group').catch(() => {})
      const label = now && now.tick !== d.tick && Math.abs(now.bpm - d.bpm) < 0.05
        ? 'Move tempo change'
        : `Tempo ${now ? now.bpm : d.bpm} BPM`
      useHistoryStore.getState().push(label, d.since)
      store.setLaneScale(null)
      tempoChanged()
    }
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
    return () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
    }
  }, [dragging]) // eslint-disable-line react-hooks/exhaustive-deps

  // A menu closes on a click anywhere else and on Escape.
  useEffect(() => {
    if (!menu) return
    const close = () => setMenu(null)
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') close() }
    window.addEventListener('mousedown', close)
    window.addEventListener('keydown', key)
    return () => {
      window.removeEventListener('mousedown', close)
      window.removeEventListener('keydown', key)
    }
  }, [menu])

  const onMouseMove = (e: React.MouseEvent) => {
    if (drag.current) return
    const { x, y } = local(e)
    const index = pointAt(entries, x, y, tickToX, scale)
    useTempoMapStore.getState().setLaneHot(index)
    setHover(index !== null)
    onSetHint?.(index === null
      ? 'Tempo lane: right-click to add a tempo change'
      : `${calloutFor(index, entries[index].tick, entries[index].bpm)}. Drag to change it, double-click to type a value, right-click for more`)
  }

  const onContextMenu = (e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    const { x, y } = local(e)
    const hit = pointAt(entries, x, y, tickToX, scale)
    const tick = snap(xToTick(x), e.altKey)
    setMenu({
      x: e.clientX, y: e.clientY, tick,
      index: hit ?? changeAt(entries, xToTick(x)),
      onPoint: hit !== null,
    })
  }

  const typeAt = (index: number, x: number, y: number) => {
    const e = useTempoMapStore.getState().entries[index]
    if (!e) return
    setTyping({ x, y, index, draft: e.bpm.toFixed(1) })
  }

  const onDoubleClick = async (e: React.MouseEvent) => {
    const { x, y } = local(e)
    const hit = pointAt(entries, x, y, tickToX, scale)
    if (hit !== null) { typeAt(hit, e.clientX, e.clientY); return }
    const added = await useTempoMapStore.getState().addTempoChange(snap(xToTick(x), e.altKey))
    if (added !== null) typeAt(added, e.clientX, e.clientY)
  }

  const commitTyping = async () => {
    const t = typing
    setTyping(null)
    if (!t) return
    const v = parseFloat(t.draft.replace(',', '.'))
    if (!Number.isFinite(v)) return
    await useTempoMapStore.getState().setEntryTempo(t.index, roundBpm(v))
  }

  const target = menu ? entries[menu.index] : undefined
  const hasNext = menu ? menu.index < entries.length - 1 : false
  const item = (label: string, onPick: () => void, opts: { disabled?: boolean; key?: string; on?: boolean } = {}) => (
    <div
      role="menuitem"
      aria-disabled={opts.disabled}
      onMouseDown={(ev) => ev.stopPropagation()}
      onClick={() => { if (opts.disabled) return; setMenu(null); onPick() }}
      style={{
        padding: '5px 9px', borderRadius: 5, color: opts.disabled ? '#52525b' : '#e4e4e7',
        cursor: opts.disabled ? 'default' : 'pointer', display: 'flex', gap: 8,
      }}
      onMouseEnter={(ev) => { if (!opts.disabled) ev.currentTarget.style.background = '#26262d' }}
      onMouseLeave={(ev) => { ev.currentTarget.style.background = 'transparent' }}
    >
      <span style={{ width: 10, color: TEMPO_COLOR }}>{opts.on ? '✓' : ''}</span>
      <span style={{ flex: 1 }}>{label}</span>
      {opts.key && <span style={{ color: '#52525b' }}>{opts.key}</span>}
    </div>
  )

  return (
    <div
      ref={ref}
      data-testid="tempo-lane"
      style={{
        position: 'absolute', left: 0, right: 0, top, height: TEMPO_LANE_HEIGHT,
        cursor: dragging ? 'grabbing' : hover ? 'grab' : 'default',
      }}
      onMouseDown={onMouseDown}
      onMouseMove={onMouseMove}
      onMouseLeave={() => { if (!drag.current) { useTempoMapStore.getState().setLaneHot(null); setHover(false) } }}
      onContextMenu={onContextMenu}
      onDoubleClick={onDoubleClick}
      onWheel={onWheel}
    >
      {callout && (
        <div
          style={{
            position: 'absolute', left: Math.max(4, callout.x - 250), top: Math.max(2, callout.y - 12),
            background: '#1c1c22', border: '1px solid rgba(245,176,65,.5)', color: '#fafafa',
            fontSize: 11.5, borderRadius: 7, padding: '6px 9px', whiteSpace: 'nowrap', pointerEvents: 'none',
            boxShadow: '0 8px 22px rgba(0,0,0,.5)', zIndex: 5,
          }}
        >
          {callout.text.split(' · ').map((part, i) => (
            <span key={i}>
              {i > 0 && ' · '}
              {/BPM$/.test(part)
                ? <em style={{ fontStyle: 'normal', color: TEMPO_COLOR, fontFamily: '"JetBrains Mono", ui-monospace, monospace' }}>{part}</em>
                : part}
            </span>
          ))}
        </div>
      )}
      {menu && (
        <div
          role="menu"
          onMouseDown={(ev) => ev.stopPropagation()}
          style={{
            position: 'fixed', left: menu.x, top: menu.y, zIndex: 1000, width: 250, whiteSpace: 'nowrap',
            background: '#17171c', border: '1px solid rgba(255,255,255,.12)', borderRadius: 8, padding: 4,
            fontSize: 12.5, boxShadow: '0 14px 34px rgba(0,0,0,.6)',
          }}
        >
          {item('Add tempo change here', () => { void useTempoMapStore.getState().addTempoChange(menu.tick) }, {
            key: barLabel(segments, menu.tick), disabled: menu.tick <= 0,
          })}
          {item('Ramp to the next change', () => { void useTempoMapStore.getState().setEntryRamp(menu.index, 'linear') }, {
            disabled: !hasNext, on: hasNext && target?.ramp === 'linear',
          })}
          {item('Jump at the next change', () => { void useTempoMapStore.getState().setEntryRamp(menu.index, 'instant') }, {
            disabled: !hasNext, on: hasNext && target?.ramp !== 'linear',
          })}
          <div style={{ borderTop: '1px solid rgba(255,255,255,.07)', margin: '4px 0' }} />
          {item('Type a tempo…', () => typeAt(menu.index, menu.x, menu.y), {
            key: target ? fmt(target.bpm) : undefined,
          })}
          {item('Delete this change', () => { void useTempoMapStore.getState().removeTempoChange(menu.index) }, {
            disabled: menu.index === 0,
          })}
          <div style={{ borderTop: '1px solid rgba(255,255,255,.07)', margin: '4px 0' }} />
          {item('Hide the tempo lane', () => useGeneralPrefsStore.getState().setTempoLaneShown(false))}
        </div>
      )}
      {typing && (
        <input
          autoFocus
          aria-label="Tempo in BPM"
          value={typing.draft}
          onChange={(ev) => setTyping({ ...typing, draft: ev.target.value })}
          onKeyDown={(ev) => {
            ev.stopPropagation()
            if (ev.key === 'Enter') void commitTyping()
            if (ev.key === 'Escape') setTyping(null)
          }}
          onBlur={() => void commitTyping()}
          onMouseDown={(ev) => ev.stopPropagation()}
          style={{
            position: 'fixed', left: typing.x + 8, top: typing.y - 12, width: 72, zIndex: 1000,
            background: '#1c1c22', color: '#fafafa', border: `1px solid ${TEMPO_COLOR}`, borderRadius: 5,
            padding: '3px 6px', font: '500 12px "JetBrains Mono", ui-monospace, monospace', outline: 'none',
          }}
        />
      )}
    </div>
  )
}
