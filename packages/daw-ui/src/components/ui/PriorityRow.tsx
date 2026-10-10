import {
  Children, isValidElement, useCallback, useEffect, useLayoutEffect, useRef, useState,
  type HTMLAttributes, type ReactElement, type ReactNode,
} from 'react'

/**
 * A toolbar row that fits any window width without squeezing or
 * overlapping. Each child is a PriorityGroup; when the row is too narrow
 * for all of them, the lowest-priority groups move, in order, into a
 * "More" menu at the end until the rest fit. Widen the window and they
 * come back. A group with priority Infinity never leaves the row.
 *
 * Measuring: every group is measured while it is in the row and its width
 * is remembered, so a group in the More menu still counts for the fit.
 * A flexible group takes the leftover space; its share of the fit is its
 * minWidth (a number, or 'content' for the width of what is inside it).
 */

export interface PriorityGroupProps {
  /** Stable name; used to remember the group's width. */
  id: string
  /** Higher stays in the row longer. Infinity never leaves it. */
  priority: number
  /** Takes the leftover room in the row. */
  flexible?: boolean
  /** What a flexible group needs at least. */
  minWidth?: number | 'content'
  /** The wrapper is a window drag region (the top bar). */
  drag?: boolean
  className?: string
  children?: ReactNode
}

/** A marker: where the More button goes. Without it, at the end. */
export function PriorityOverflowSlot() { return null }

export function PriorityGroup(_props: PriorityGroupProps) {
  // Rendered by PriorityRow, which reads the props; never on its own.
  return null
}

interface Props extends HTMLAttributes<HTMLDivElement> {
  /** What the More button says on hover. */
  moreTitle?: string
  children?: ReactNode
}

const MORE_WIDTH = 34

export function PriorityRow({ children, className, moreTitle = 'More', ...rest }: Props) {
  const rowRef = useRef<HTMLDivElement>(null)
  const widths = useRef(new Map<string, number>())
  const groupEls = useRef(new Map<string, HTMLDivElement>())
  const [hidden, setHidden] = useState<string[]>([])
  const [open, setOpen] = useState(false)

  const all = Children.toArray(children).filter(isValidElement) as ReactElement[]
  const groups = all.filter((c) => c.type === PriorityGroup) as ReactElement<PriorityGroupProps>[]
  const slotIndex = all.findIndex((c) => c.type === PriorityOverflowSlot)

  const measure = useCallback(() => {
    const row = rowRef.current
    if (!row) return
    const style = getComputedStyle(row)
    const gap = parseFloat(style.columnGap || style.gap) || 0
    const avail = row.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight)
    for (const g of groups) {
      const el = groupEls.current.get(g.props.id)
      if (!el) continue
      const p = g.props
      if (p.flexible) {
        widths.current.set(p.id, p.minWidth === 'content'
          ? (el.firstElementChild as HTMLElement | null)?.scrollWidth ?? 0
          : p.minWidth ?? 0)
      } else {
        widths.current.set(p.id, el.offsetWidth)
      }
    }
    const w = (id: string) => widths.current.get(id) ?? 0
    const visible = groups.map((g) => g.props.id)
    const total = (ids: string[]) => ids.reduce((s, id) => s + w(id), 0) + Math.max(0, ids.length - 1) * gap
    let next: string[] = []
    if (total(visible) > avail) {
      const order = [...groups].sort((a, b) => a.props.priority - b.props.priority)
      let shown = [...visible]
      for (const g of order) {
        if (total(shown) + MORE_WIDTH + gap <= avail) break
        if (!Number.isFinite(g.props.priority)) break
        shown = shown.filter((id) => id !== g.props.id)
        next.push(g.props.id)
      }
      // Room may be left once a wide group has gone: bring back any
      // narrower group that fits, the most important first.
      for (const g of [...order].reverse()) {
        if (!next.includes(g.props.id)) continue
        if (total([...shown, g.props.id]) + MORE_WIDTH + gap <= avail) {
          shown.push(g.props.id)
          next = next.filter((id) => id !== g.props.id)
        }
      }
      next = groups.map((g) => g.props.id).filter((id) => next.includes(id))
    }
    setHidden((cur) => (cur.length === next.length && cur.every((id, i) => id === next[i]) ? cur : next))
  }, [groups])

  useLayoutEffect(() => { measure() })

  useEffect(() => {
    const row = rowRef.current
    if (!row) return
    const ro = new ResizeObserver(() => measure())
    ro.observe(row)
    for (const el of groupEls.current.values()) ro.observe(el)
    return () => ro.disconnect()
  }, [measure, hidden])

  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      if (rowRef.current && !rowRef.current.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setOpen(false) }
    window.addEventListener('mousedown', onDown)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDown)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  // Nothing left to hide: the menu closes on its own.
  useEffect(() => { if (hidden.length === 0) setOpen(false) }, [hidden.length])

  const renderGroup = (g: ReactElement<PriorityGroupProps>, inMenu: boolean) => {
    const p = g.props
    return (
      <div
        key={p.id}
        ref={(el) => { if (el && !inMenu) groupEls.current.set(p.id, el); else if (!inMenu) groupEls.current.delete(p.id) }}
        className={`hw-pgroup${p.flexible && !inMenu ? ' flexible' : ''}${p.className ? ' ' + p.className : ''}`}
        data-group={p.id}
        {...(p.drag && !inMenu ? { 'data-tauri-drag-region': true } : {})}
      >
        {p.children}
      </div>
    )
  }

  const more = hidden.length > 0 ? (
    <div className="hw-prow-more-host" key="__more">
      <button
        type="button"
        className={`hw-prow-more${open ? ' on' : ''}`}
        title={moreTitle}
        aria-label={moreTitle}
        aria-expanded={open}
        onMouseDown={(e) => e.stopPropagation()}
        onClick={() => setOpen((v) => !v)}
      >
        <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
          <circle cx="2.5" cy="7" r="1.4" fill="currentColor" /><circle cx="7" cy="7" r="1.4" fill="currentColor" /><circle cx="11.5" cy="7" r="1.4" fill="currentColor" />
        </svg>
      </button>
      {open && (
        <div className="hw-prow-pop" data-overflow onMouseDown={(e) => e.stopPropagation()}>
          {groups.filter((g) => hidden.includes(g.props.id)).map((g) => renderGroup(g, true))}
        </div>
      )}
    </div>
  ) : null

  const inline: ReactNode[] = []
  all.forEach((c, i) => {
    if (c.type === PriorityOverflowSlot) { if (more) inline.push(more); return }
    if (c.type === PriorityGroup) {
      const p = (c as ReactElement<PriorityGroupProps>).props
      if (!hidden.includes(p.id)) inline.push(renderGroup(c as ReactElement<PriorityGroupProps>, false))
      return
    }
    inline.push(<span key={`c${i}`}>{c}</span>)
  })
  if (slotIndex < 0 && more) inline.push(more)

  return (
    <div ref={rowRef} className={`hw-prow${className ? ' ' + className : ''}`} {...rest}>
      {inline}
    </div>
  )
}
