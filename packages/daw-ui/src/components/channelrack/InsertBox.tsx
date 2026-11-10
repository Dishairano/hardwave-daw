import { memo, useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { useTrackStore } from '../../stores/trackStore'
import {
  insertBoxText,
  insertName,
  insertOf,
  insertsByNumber,
  lastMixerNumber,
  parseInsertNumber,
  stepNumber,
} from '../../lib/mixerNumbers'
import { routeToInsert } from '../../lib/insertActions'
import { t, tf, useLanguage } from '../../i18n'
import './insertBox.css'

/** Pixels of drag per number, as FL's insert box steps. */
const DRAG_STEP_PX = 8
/** How long after the last wheel step the number is taken. */
const WHEEL_SETTLE_MS = 450

/**
 * The channel rack's insert box (mockup M3): the insert a channel plays
 * into, by the number the mixer shows (M for the master). A click opens
 * the list of inserts by number and name, then the empty numbers; picking
 * one routes the channel there, and an empty number becomes an insert.
 * Dragging up or down, or scrolling, on the box steps through the numbers,
 * as in FL; the number is taken when the drag ends or the wheel settles,
 * so a sweep is one undo step. An automation channel makes no sound and
 * has a dashed box with a dash.
 *
 * It showed the channel's place among all tracks before, which was the
 * mixer's count of its strips but not the number the sample window read.
 */
export const InsertBox = memo(function InsertBox({ trackId, name }: { trackId: string; name: string }) {
  useLanguage()
  const number = useTrackStore((s) => {
    const track = s.tracksById[trackId]
    return track ? insertOf(track, s.tracksById) : null
  })
  const automation = useTrackStore((s) => s.tracksById[trackId]?.kind === 'Automation')
  const last = useTrackStore((s) => lastMixerNumber(s.tracks))
  const [preview, setPreview] = useState<number | null>(null)
  const [open, setOpen] = useState<DOMRect | null>(null)
  const boxRef = useRef<HTMLButtonElement>(null)
  const wheelTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const drag = useRef<{ y: number; from: number; moved: boolean } | null>(null)
  const previewRef = useRef<number | null>(null)
  const pick = (n: number) => {
    setOpen(null)
    setPreview(null)
    previewRef.current = null
    if (n === number) return
    void routeToInsert([trackId], n, tf('Route "{name}" to insert {n}', { name, n: insertBoxText(n) })).catch(() => {})
  }
  // The wheel steps through numbers on the box instead of scrolling the
  // rack, so it is a listener of its own that may stop the scroll (React's
  // wheel events cannot). The number is taken once the wheel settles.
  const latest = useRef({ number, last, pick })
  latest.current = { number, last, pick }
  const hasBox = !automation && number != null
  useEffect(() => {
    const el = boxRef.current
    if (!hasBox || !el) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      e.stopPropagation()
      const { number: now, last: end } = latest.current
      const next = stepNumber(previewRef.current ?? now ?? 0, e.deltaY < 0 ? 1 : -1, end)
      previewRef.current = next
      setPreview(next)
      if (wheelTimer.current) clearTimeout(wheelTimer.current)
      wheelTimer.current = setTimeout(() => latest.current.pick(next), WHEEL_SETTLE_MS)
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => {
      el.removeEventListener('wheel', onWheel)
      if (wheelTimer.current) clearTimeout(wheelTimer.current)
    }
  }, [hasBox])

  if (automation || number == null) {
    return (
      <span className="cr-ins none" title={automation ? t('No insert: an automation channel makes no sound') : t('Plays into a track that is not in the mixer')}>
        {insertBoxText(null)}
      </span>
    )
  }

  const shown = preview ?? number
  const title = shown === 0
    ? tf('{track} plays into the master. Click for the inserts, or drag or scroll to step through them', { track: name })
    : tf('{track} plays into insert {n}. Click for the inserts, or drag or scroll to step through them', { track: name, n: shown })

  return (
    <>
      <button
        ref={boxRef}
        type="button"
        className={'cr-ins' + (shown === 0 ? ' m' : '') + (open ? ' open' : '') + (preview != null ? ' stepping' : '')}
        title={title}
        data-testid="insert-box"
        onPointerDown={(e) => {
          if (e.button !== 0) return
          e.stopPropagation()
          drag.current = { y: e.clientY, from: number, moved: false }
          ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
        }}
        onPointerMove={(e) => {
          const d = drag.current
          if (!d) return
          const steps = Math.trunc((d.y - e.clientY) / DRAG_STEP_PX)
          if (steps === 0 && !d.moved) return
          d.moved = true
          previewRef.current = stepNumber(d.from, steps, last)
          setPreview(previewRef.current)
        }}
        onPointerUp={(e) => {
          const d = drag.current
          drag.current = null
          ;(e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId)
          if (!d) return
          if (d.moved) {
            const to = stepNumber(d.from, Math.trunc((d.y - e.clientY) / DRAG_STEP_PX), last)
            pick(to)
          } else {
            setOpen((o) => (o ? null : boxRef.current?.getBoundingClientRect() ?? null))
          }
        }}
      >
        {insertBoxText(shown)}
      </button>
      {open && <InsertList anchor={open} current={number} last={last} onPick={pick} onClose={() => setOpen(null)} />}
    </>
  )
})

/** The list under the box: the master, the inserts by number and name, the
 *  empty numbers dimmed, and a field to type a number into. */
function InsertList({ anchor, current, last, onPick, onClose }: {
  anchor: DOMRect
  current: number
  last: number
  onPick: (n: number) => void
  onClose: () => void
}) {
  useLanguage()
  const tracks = useTrackStore((s) => s.tracks)
  const byNumber = useMemo(() => insertsByNumber(tracks), [tracks])
  const [typed, setTyped] = useState('')
  const listRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    inputRef.current?.focus()
    // The current insert in view.
    listRef.current?.querySelector('.cur')?.scrollIntoView({ block: 'center' })
    const close = (e: MouseEvent) => {
      if ((e.target as HTMLElement).closest('.cr-ins-pop, .cr-ins')) return
      onClose()
    }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [onClose])

  const numbers = Array.from({ length: last + 1 }, (_, n) => n)
  const left = Math.min(anchor.left, window.innerWidth - 330)
  const top = Math.min(anchor.bottom + 4, window.innerHeight - 380)
  return createPortal(
    <div className="cr-ins-pop" style={{ left, top }} onContextMenu={(e) => e.preventDefault()}>
      <div className="cr-ins-pop-head">
        {t('PLAYS INTO')}
        <input
          ref={inputRef}
          value={typed}
          placeholder={insertBoxText(current)}
          aria-label={t('Insert number')}
          onChange={(e) => setTyped(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation()
            if (e.key === 'Escape') onClose()
            if (e.key !== 'Enter') return
            const n = parseInsertNumber(typed, last)
            if (n != null) onPick(n)
          }}
        />
      </div>
      <div className="cr-ins-pop-list" ref={listRef}>
        {numbers.map((n) => {
          const track = n === 0 ? null : byNumber.get(n)
          const empty = n !== 0 && !track
          const colour = n === 0 ? '#DC2626' : track ? (/^#[0-9a-f]{6}$/i.test(track.color) ? track.color : '#52525b') : 'transparent'
          return (
            <div
              key={n}
              role="option"
              aria-selected={n === current}
              className={(n === current ? 'cur ' : '') + (empty ? 'emp' : '')}
              onClick={() => onPick(n)}
            >
              <b>{insertBoxText(n)}</b>
              <i style={{ background: colour }} />
              <span>{insertName(n, track)}</span>
              {n === current && <small>{t('now')}</small>}
            </div>
          )
        })}
      </div>
      <div className="cr-ins-pop-foot">
        {tf('Type a number, or scroll the list (to {last}). The dimmed numbers are empty: picking one makes the insert. On the box itself, drag or scroll to step through numbers.', { last })}
      </div>
    </div>,
    document.body,
  )
}
