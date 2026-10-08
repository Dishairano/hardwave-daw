import { useEffect, type RefObject } from 'react'

/**
 * Dragging inside the app with pointer events instead of HTML5 drag and drop.
 *
 * On Windows the window's native file-drop handler (needed so files dropped
 * from Explorer arrive with their real paths) takes WebView2's drop target,
 * and HTML5 drag events inside the page never fire. Dragging a sample from
 * the browser onto the playlist did nothing there. Pointer events are not
 * affected, so in-app drags are done with them: the source starts a drag,
 * the element under the pointer receives `hw-dragover`, `hw-dragleave` and
 * `hw-drop` events, and targets subscribe with `useDropTarget`.
 *
 * The data is the same string the HTML5 drags carried ("file:<path>",
 * "folder:<id>").
 */

export interface HwDragDetail {
  data: string
  clientX: number
  clientY: number
  /** For hw-dragleave: the element the pointer moved to. */
  next?: Element | null
}

const THRESHOLD_PX = 4

/** The drag in progress, if any; read by targets to decide whether to light up. */
let current: string | null = null
export function currentDrag(): string | null {
  return current
}

/** Start a drag from a pointer press. Nothing happens until the pointer has
 *  moved a few pixels, so a click stays a click. */
export function beginPointerDrag(e: React.PointerEvent, data: string, label: string): void {
  if (e.button !== 0) return
  const startX = e.clientX
  const startY = e.clientY
  let active = false
  let ghost: HTMLDivElement | null = null
  let over: Element | null = null

  const fire = (target: Element | null, type: string, detail: HwDragDetail) => {
    target?.dispatchEvent(new CustomEvent<HwDragDetail>(type, { bubbles: true, detail }))
  }

  const move = (ev: PointerEvent) => {
    if (!active) {
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < THRESHOLD_PX) return
      active = true
      current = data
      ghost = document.createElement('div')
      ghost.textContent = label
      Object.assign(ghost.style, {
        position: 'fixed', left: '0', top: '0', zIndex: '10000', pointerEvents: 'none',
        padding: '3px 8px', borderRadius: '4px', fontSize: '11px',
        background: 'rgba(12,12,17,0.92)', color: '#fff',
        border: '1px solid rgba(255,255,255,0.15)', whiteSpace: 'nowrap',
      } satisfies Partial<CSSStyleDeclaration>)
      document.body.appendChild(ghost)
      document.body.style.cursor = 'copy'
    }
    if (ghost) ghost.style.transform = `translate(${ev.clientX + 12}px, ${ev.clientY + 10}px)`
    const target = document.elementFromPoint(ev.clientX, ev.clientY)
    if (target !== over) {
      fire(over, 'hw-dragleave', { data, clientX: ev.clientX, clientY: ev.clientY, next: target })
      over = target
    }
    fire(target, 'hw-dragover', { data, clientX: ev.clientX, clientY: ev.clientY })
  }

  const finish = (ev: PointerEvent | null, drop: boolean) => {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', up)
    window.removeEventListener('pointercancel', cancel)
    window.removeEventListener('keydown', key)
    ghost?.remove()
    document.body.style.cursor = ''
    current = null
    if (!active) return
    const x = ev?.clientX ?? 0
    const y = ev?.clientY ?? 0
    fire(over, 'hw-dragleave', { data, clientX: x, clientY: y, next: null })
    if (drop && ev) fire(document.elementFromPoint(x, y), 'hw-drop', { data, clientX: x, clientY: y })
  }
  const up = (ev: PointerEvent) => finish(ev, true)
  const cancel = () => finish(null, false)
  const key = (ev: KeyboardEvent) => { if (ev.key === 'Escape') finish(null, false) }

  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', up)
  window.addEventListener('pointercancel', cancel)
  window.addEventListener('keydown', key)
}

export interface DropTargetHandlers {
  /** Whether this target takes the dragged data. Default: everything. */
  accepts?: (data: string) => boolean
  onOver?: (detail: HwDragDetail) => void
  onLeave?: () => void
  onDrop: (detail: HwDragDetail) => void
}

/** Make an element a drop target for pointer drags. The innermost target
 *  under the pointer takes the drop; its parents do not see it. */
export function useDropTarget(ref: RefObject<HTMLElement | null>, handlers: DropTargetHandlers): void {
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const takes = (d: HwDragDetail) => handlers.accepts?.(d.data) ?? true
    const over = (ev: Event) => {
      const d = (ev as CustomEvent<HwDragDetail>).detail
      if (!takes(d)) return
      ev.stopPropagation()
      handlers.onOver?.(d)
    }
    const leave = (ev: Event) => {
      const d = (ev as CustomEvent<HwDragDetail>).detail
      if (d.next && el.contains(d.next)) return
      handlers.onLeave?.()
    }
    const drop = (ev: Event) => {
      const d = (ev as CustomEvent<HwDragDetail>).detail
      if (!takes(d)) return
      ev.stopPropagation()
      handlers.onLeave?.()
      handlers.onDrop(d)
    }
    el.addEventListener('hw-dragover', over)
    el.addEventListener('hw-dragleave', leave)
    el.addEventListener('hw-drop', drop)
    return () => {
      el.removeEventListener('hw-dragover', over)
      el.removeEventListener('hw-dragleave', leave)
      el.removeEventListener('hw-drop', drop)
    }
  })
}
