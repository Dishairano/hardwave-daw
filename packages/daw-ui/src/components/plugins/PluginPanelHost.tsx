import { useEffect, useRef, useState } from 'react'
import type { PointerEvent as ReactPointerEvent } from 'react'
import { createPortal } from 'react-dom'
import { HwPluginWindow } from './HwPluginWindow'
import { PluginControls } from '../mixer/PluginControls'
import { layoutFor, windowWidth } from './pluginLayouts'
import { OPEN_PLUGIN_PANEL, type PluginWindowRequest } from './openPluginWindow'

interface OpenPanel {
  req: PluginWindowRequest
  x: number
  y: number
  z: number
}

/**
 * The plug-in windows that float over the app beside the docked mixer, one
 * per slot. Each can be dragged by its header, is raised on click and closes
 * with its × or Esc (the one in front).
 */
export function PluginPanelHost() {
  const [panels, setPanels] = useState<OpenPanel[]>([])
  const zTop = useRef(6000)

  useEffect(() => {
    const onOpen = (e: Event) => {
      const req = (e as CustomEvent<PluginWindowRequest>).detail
      setPanels((list) => {
        const z = ++zTop.current
        const existing = list.find((p) => p.req.slotId === req.slotId && p.req.trackId === req.trackId)
        if (existing) return list.map((p) => (p === existing ? { ...p, z } : p))
        const layout = layoutFor(req.pluginId)
        const w = Math.min(layout ? windowWidth(layout) : 340, window.innerWidth - 32)
        const n = list.length
        return [...list, { req, z, x: Math.max(16, (window.innerWidth - w) / 2 + n * 24), y: 72 + n * 24 }]
      })
    }
    window.addEventListener(OPEN_PLUGIN_PANEL, onOpen)
    return () => window.removeEventListener(OPEN_PLUGIN_PANEL, onOpen)
  }, [])

  useEffect(() => {
    if (!panels.length) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      setPanels((list) => {
        if (!list.length) return list
        const top = list.reduce((a, b) => (b.z > a.z ? b : a))
        return list.filter((p) => p !== top)
      })
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [panels.length])

  const close = (p: OpenPanel) => setPanels((list) => list.filter((x) => x !== p))
  const raise = (p: OpenPanel) => setPanels((list) => list.map((x) => (x === p ? { ...x, z: ++zTop.current } : x)))
  const drag = (p: OpenPanel) => (e: ReactPointerEvent) => {
    if (e.button !== 0) return
    const x0 = e.clientX - p.x, y0 = e.clientY - p.y
    const move = (ev: PointerEvent) => {
      const x = Math.max(-200, Math.min(window.innerWidth - 120, ev.clientX - x0))
      const y = Math.max(0, Math.min(window.innerHeight - 40, ev.clientY - y0))
      setPanels((list) => list.map((q) => (q.req.slotId === p.req.slotId && q.req.trackId === p.req.trackId ? { ...q, x, y } : q)))
    }
    const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up) }
    window.addEventListener('pointermove', move); window.addEventListener('pointerup', up)
  }

  if (!panels.length) return null
  return createPortal(
    <>
      {panels.map((p) => {
        const layout = layoutFor(p.req.pluginId)
        const key = `${p.req.trackId}:${p.req.slotId}`
        return (
          <div
            key={key}
            role="dialog"
            aria-label={`${p.req.pluginName} window`}
            onPointerDownCapture={() => raise(p)}
            style={{
              position: 'fixed', left: p.x, top: p.y, zIndex: p.z,
              width: layout ? Math.min(windowWidth(layout), window.innerWidth - 32) : 'min(340px, calc(100vw - 32px))',
              maxHeight: 'calc(100vh - 24px)', overflow: 'auto', borderRadius: 10,
              ...(layout ? {} : {
                display: 'flex', flexDirection: 'column', background: '#0c0c11',
                border: '1px solid rgba(255,255,255,0.1)', boxShadow: '0 12px 40px rgba(0,0,0,0.6)',
              }),
            }}
          >
            {layout ? (
              <HwPluginWindow
                trackId={p.req.trackId} slotId={p.req.slotId} pluginId={p.req.pluginId} pluginName={p.req.pluginName}
                onClose={() => close(p)} onHeaderPointerDown={drag(p)}
              />
            ) : (
              <>
                <div onPointerDown={drag(p)} style={{ height: 8, cursor: 'move' }} />
                <PluginControls
                  trackId={p.req.trackId} slotId={p.req.slotId} pluginId={p.req.pluginId} pluginName={p.req.pluginName}
                  onClose={() => close(p)}
                />
              </>
            )}
          </div>
        )
      })}
    </>,
    document.body,
  )
}
