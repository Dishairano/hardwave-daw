import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { clampRect, type PanelKey, type Rect } from '../../lib/workspaceLayout'
import { useWorkspaceStore } from '../../stores/workspaceStore'
import { PanelFrame, type DragHints } from './PanelFrame'
import './workspace.css'

/**
 * The work area under the tool rows: every shown panel as a window that
 * moves, resizes, docks and detaches (see PanelFrame). The places are kept
 * per computer by the workspace store; which panels show stays with the
 * panel toggles.
 */

export interface WorkspacePanel {
  key: PanelKey
  title: string
  shown: boolean
  head?: ReactNode
  onClose: () => void
  onDetach?: () => void
  testId?: string
  content: ReactNode
}

export function Workspace({ panels }: { panels: WorkspacePanel[] }) {
  const ref = useRef<HTMLDivElement>(null)
  const area = useWorkspaceStore((s) => s.area)
  const places = useWorkspaceStore((s) => s.panels)
  const setArea = useWorkspaceStore((s) => s.setArea)
  const [hints, setHints] = useState<DragHints | null>(null)

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const measure = () => setArea({ w: el.clientWidth, h: el.clientHeight })
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [setArea])

  // Esc lets go of a stuck drag hint (a pointer released outside the window).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setHints(null) }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const shown = panels.filter((p) => p.shown)
  const rectOf = (k: PanelKey): Rect | null => {
    if (!places || !area) return null
    const g = places[k]
    return g.max ? null : clampRect(g, k, area)
  }

  return (
    <div className="hw-ws" ref={ref} data-testid="workspace">
      {places && area && shown.map((p) => (
        <PanelFrame
          key={p.key}
          id={p.key}
          title={p.title}
          head={p.head}
          geometry={places[p.key]}
          area={area}
          others={shown.filter((o) => o.key !== p.key).map((o) => rectOf(o.key)).filter((r): r is Rect => !!r)}
          onHints={setHints}
          onClose={p.onClose}
          onDetach={p.onDetach}
          testId={p.testId}
        >
          {p.content}
        </PanelFrame>
      ))}
      {hints?.guides.map((g, i) => (
        <div
          key={i}
          className="hw-ws-guide"
          style={g.axis === 'x' ? { left: g.at - 1, top: 0, width: 2, bottom: 0 } : { top: g.at - 1, left: 0, height: 2, right: 0 }}
        />
      ))}
      {hints?.zone && (
        <div className="hw-ws-zone" style={{ left: hints.zone.x + 4, top: hints.zone.y + 4, width: hints.zone.w - 8, height: hints.zone.h - 8 }} />
      )}
      {shown.length === 0 && (
        <div className="hw-ws-empty">No panels open. F5 Playlist · F6 Channel rack · F7 Piano roll · F8 Browser · F9 Mixer</div>
      )}
    </div>
  )
}
