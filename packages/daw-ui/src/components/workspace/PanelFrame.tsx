import { useEffect, useRef, useState, type ReactNode } from 'react'
import {
  clampRect, dockRect, dockZone, snapMove, snapResize,
  type Area, type DockSide, type Guide, type PanelGeometry, type PanelKey, type Rect, type ResizeDir,
} from '../../lib/workspaceLayout'
import { useWorkspaceStore } from '../../stores/workspaceStore'
import { t, useLanguage } from '../../i18n'

/**
 * One panel in the work area, as in FL: a title bar to drag it by, edges
 * and corners to resize it from, its own menu with "Detached" (own OS
 * window), maximize, dock to a side, and close. While it moves it snaps to
 * the edges and to the other panels, and dropping it against an edge docks
 * it there. Approved mockup: suite.hardwavestudios.com/daw-panels-mockup/.
 */

const DIRS: ResizeDir[] = ['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw']

/** What the work area draws while a panel moves: guides and the dock zone. */
export interface DragHints { guides: Guide[]; zone: Rect | null }

interface Props {
  id: PanelKey
  title: string
  /** The panel's own header content (the playlist's arrangement and
   *  pattern), shown in the title bar instead of the plain title. */
  head?: ReactNode
  geometry: PanelGeometry
  area: Area
  /** The other shown panels, for snapping. */
  others: Rect[]
  onHints: (h: DragHints | null) => void
  onClose: () => void
  /** Open in its own OS window; absent where that is not possible. */
  onDetach?: () => void
  testId?: string
  children: ReactNode
}

export function PanelFrame({ id, title, head, geometry, area, others, onHints, onClose, onDetach, testId, children }: Props) {
  useLanguage()
  const setRect = useWorkspaceStore((s) => s.setRect)
  const dock = useWorkspaceStore((s) => s.dock)
  const float = useWorkspaceStore((s) => s.float)
  const front = useWorkspaceStore((s) => s.front)
  const toggleMax = useWorkspaceStore((s) => s.toggleMax)
  // While dragging, the frame follows the pointer from this local copy and
  // the store is written once, on release.
  const [live, setLive] = useState<Rect | null>(null)
  const [menu, setMenu] = useState(false)
  const frameRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!menu) return
    const close = (e: PointerEvent) => {
      if (!(e.target as HTMLElement).closest?.('.hw-panel-menu, .hw-panel-menu-btn')) setMenu(false)
    }
    window.addEventListener('pointerdown', close)
    return () => window.removeEventListener('pointerdown', close)
  }, [menu])

  const rect: Rect = geometry.max
    ? { x: 0, y: 0, w: area.w, h: area.h }
    : live ?? clampRect(geometry, id, area)

  const track = (e: React.PointerEvent, onMove: (ev: PointerEvent) => void, onUp: () => void) => {
    const target = e.currentTarget as HTMLElement
    target.setPointerCapture?.(e.pointerId)
    const move = (ev: PointerEvent) => onMove(ev)
    const up = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', up)
      onUp()
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', up)
  }

  const startDrag = (e: React.PointerEvent) => {
    if (e.button !== 0) return
    if ((e.target as HTMLElement).closest('button, select, input, textarea, [data-nodrag]')) return
    // No preventDefault here: it would swallow the double-click that
    // maximizes. The bar does not select text (user-select: none).
    front(id)
    const box = frameRef.current?.parentElement?.getBoundingClientRect()
    if (!box) return
    // Pulling a maximized or docked panel loose: it comes away at a free
    // size, centred under the pointer, as a window does.
    let start: Rect = rect
    if (geometry.max || geometry.dock) {
      const w = Math.round(Math.min(rect.w, area.w * 0.55)), h = Math.round(Math.min(rect.h, area.h * 0.6))
      start = { x: Math.round(e.clientX - box.left - w / 2), y: Math.max(0, Math.round(e.clientY - box.top - 12)), w, h }
    }
    const from = { x: e.clientX, y: e.clientY }
    let side: DockSide | null = null
    let moved = false
    let current = start
    track(e, (ev) => {
      if (!moved && Math.abs(ev.clientX - from.x) + Math.abs(ev.clientY - from.y) < 3) return
      moved = true
      const raw = { ...start, x: start.x + ev.clientX - from.x, y: start.y + ev.clientY - from.y }
      const { rect: snapped, guides } = snapMove(raw, others, area)
      side = dockZone(ev.clientX - box.left, ev.clientY - box.top, area)
      current = snapped
      setLive(snapped)
      onHints({ guides, zone: side ? dockRect(side, snapped, area) : null })
    }, () => {
      onHints(null)
      setLive(null)
      if (!moved) return
      if (side) dock(id, side)
      else setRect(id, clampRect(current, id, area), null)
    })
  }

  const startResize = (dir: ResizeDir) => (e: React.PointerEvent) => {
    if (e.button !== 0) return
    e.preventDefault()
    e.stopPropagation()
    front(id)
    const start = rect
    const from = { x: e.clientX, y: e.clientY }
    let current = start
    track(e, (ev) => {
      const { rect: r, guides } = snapResize(start, dir, ev.clientX - from.x, ev.clientY - from.y, id, others, area)
      current = r
      setLive(r)
      onHints({ guides, zone: null })
    }, () => {
      onHints(null)
      setLive(null)
      // A resized docked panel stays docked when it still spans its side.
      const stillDocked = geometry.dock && (
        ((geometry.dock === 'left' || geometry.dock === 'right') && current.y === 0 && current.h === area.h)
        || ((geometry.dock === 'top' || geometry.dock === 'bottom') && current.x === 0 && current.w === area.w))
      setRect(id, clampRect(current, id, area), stillDocked ? geometry.dock : null)
    })
  }

  const pick = (action: () => void) => () => { setMenu(false); action() }
  const docked = !!geometry.dock && !geometry.max

  return (
    <div
      ref={frameRef}
      className={`hw-panel${docked ? ' docked' : ''}${geometry.max ? ' max' : ''}`}
      data-testid={testId}
      data-panel={id}
      style={{ left: rect.x, top: rect.y, width: rect.w, height: rect.h, zIndex: geometry.max ? 500 + geometry.z : geometry.z }}
      onPointerDownCapture={() => front(id)}
    >
      <div
        className="hw-panel-bar"
        onPointerDown={startDrag}
        onDoubleClick={(e) => {
          if (!(e.target as HTMLElement).closest('button, select, input, [data-nodrag]')) toggleMax(id)
        }}
      >
        <button type="button" className="hw-panel-menu-btn" title={t('Panel menu')} onClick={() => setMenu((m) => !m)}>
          <svg width="11" height="11" viewBox="0 0 16 16" fill="none"><path d="M3 4.5h10M3 8h10M3 11.5h10" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" /></svg>
        </button>
        <span className="hw-panel-title">{head ?? t(title)}</span>
        {(docked || geometry.max) && <span className="hw-panel-tag">{geometry.max ? t('max') : t('docked')}</span>}
        <span className="hw-panel-sp" />
        {onDetach && (
          <button type="button" title={t('Detached: open in its own window (another screen)')} onClick={onDetach}>
            <svg width="11" height="11" viewBox="0 0 16 16" fill="none"><rect x="2.5" y="5.5" width="8" height="8" rx="1" stroke="currentColor" strokeWidth="1.3" /><path d="M6 2.5h7.5V10" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" /></svg>
          </button>
        )}
        <button type="button" title={geometry.max ? t('Restore') : t('Maximize')} onClick={() => toggleMax(id)}>
          <svg width="10" height="10" viewBox="0 0 16 16" fill="none"><rect x="3" y="3" width="10" height="10" stroke="currentColor" strokeWidth="1.4" /></svg>
        </button>
        <button type="button" className="x" title={t('Close')} onClick={onClose}>
          <svg width="10" height="10" viewBox="0 0 16 16" fill="none"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" /></svg>
        </button>
      </div>
      {menu && (
        <div className="hw-panel-menu" role="menu">
          {onDetach && <button type="button" className="hl" onClick={pick(onDetach)}>{t('Detached')}</button>}
          <button type="button" onClick={pick(() => toggleMax(id))}>{geometry.max ? t('Restore') : t('Maximize')}</button>
          <hr />
          <button type="button" onClick={pick(() => dock(id, 'left'))}>{t('Dock left')}</button>
          <button type="button" onClick={pick(() => dock(id, 'right'))}>{t('Dock right')}</button>
          <button type="button" onClick={pick(() => dock(id, 'top'))}>{t('Dock top')}</button>
          <button type="button" onClick={pick(() => dock(id, 'bottom'))}>{t('Dock bottom')}</button>
          <button type="button" onClick={pick(() => float(id))}>{t('Float')}</button>
          <hr />
          <button type="button" onClick={pick(onClose)}>{t('Close')}</button>
        </div>
      )}
      <div className="hw-panel-body">{children}</div>
      {!geometry.max && DIRS.map((d) => (
        <div key={d} className={`hw-panel-h ${d}`} onPointerDown={startResize(d)} />
      ))}
    </div>
  )
}
