import { useEffect, useRef, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { hw } from '../theme'
import { usePanelLayoutStore, type PanelId } from '../stores/panelLayoutStore'
import { useTrackStore } from '../stores/trackStore'
import { useIsMobile } from '../hooks/useIsMobile'

interface Props {
  panelId: PanelId
  title: string
  onClose?: () => void
  children: ReactNode
}

const MIN_W = 260
const MIN_H = 160

export function FloatingWindow({ panelId, title, onClose, children }: Props) {
  const layout = usePanelLayoutStore(s => s.layout[panelId])
  const setPos = usePanelLayoutStore(s => s.setPos)
  const setSize = usePanelLayoutStore(s => s.setSize)
  const setFloating = usePanelLayoutStore(s => s.setFloating)
  const bringToFront = usePanelLayoutStore(s => s.bringToFront)

  const isMobile = useIsMobile()
  const dragRef = useRef<{ startX: number; startY: number; origX: number; origY: number } | null>(null)
  const resizeRef = useRef<{ startX: number; startY: number; origW: number; origH: number } | null>(null)

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (dragRef.current) {
        const d = dragRef.current
        const nx = d.origX + (e.clientX - d.startX)
        const ny = d.origY + (e.clientY - d.startY)
        const maxX = Math.max(0, window.innerWidth - 80)
        const maxY = Math.max(0, window.innerHeight - 40)
        setPos(panelId, Math.max(0, Math.min(maxX, nx)), Math.max(28, Math.min(maxY, ny)))
      }
      if (resizeRef.current) {
        const r = resizeRef.current
        const nw = Math.max(MIN_W, r.origW + (e.clientX - r.startX))
        const nh = Math.max(MIN_H, r.origH + (e.clientY - r.startY))
        setSize(panelId, nw, nh)
      }
    }
    const onUp = () => { dragRef.current = null; resizeRef.current = null }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
  }, [panelId, setPos, setSize])

  const startDrag = (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest('[data-nodrag]')) return
    bringToFront(panelId)
    dragRef.current = { startX: e.clientX, startY: e.clientY, origX: layout.x, origY: layout.y }
    e.preventDefault()
  }

  const startResize = (e: React.MouseEvent) => {
    bringToFront(panelId)
    resizeRef.current = { startX: e.clientX, startY: e.clientY, origW: layout.w, origH: layout.h }
    e.preventDefault()
    e.stopPropagation()
  }

  const framePos = isMobile
    ? { left: 0, top: 0, width: '100vw', height: '100vh', borderRadius: 0, border: 'none' }
    : { left: layout.x, top: layout.y, width: layout.w, height: layout.h, borderRadius: hw.radius.lg, border: `1px solid ${hw.borderDark}` }

  return createPortal(
    <div
      style={{
        position: 'fixed',
        ...framePos,
        zIndex: layout.zIndex,
        background: hw.bg,
        boxShadow: isMobile ? 'none' : '0 12px 32px rgba(0,0,0,0.55)',
        display: 'flex', flexDirection: 'column',
        overflow: 'hidden',
      }}
      onMouseDown={() => bringToFront(panelId)}
    >
      <div
        onMouseDown={isMobile ? undefined : startDrag}
        style={{
          height: isMobile ? 40 : 24, flexShrink: 0,
          display: 'flex', alignItems: 'center', gap: 6,
          padding: isMobile ? '0 8px 0 14px' : '0 6px 0 10px',
          background: 'rgba(255,255,255,0.04)',
          borderBottom: `1px solid ${hw.border}`,
          cursor: isMobile ? 'default' : 'move', userSelect: 'none',
          fontSize: isMobile ? 13 : 10, color: hw.textSecondary, fontWeight: 600,
        }}
      >
        <span style={{ flex: 1 }}>{title}</span>
        {!isMobile && (
          <button
            data-nodrag
            onClick={() => setFloating(panelId, false)}
            title="Dock"
            style={{
              width: 18, height: 18, padding: 0,
              background: 'transparent', border: 'none',
              color: hw.textFaint, fontSize: 11, cursor: 'pointer',
            }}
          >⧉</button>
        )}
        {onClose && (
          <button
            data-nodrag
            onClick={onClose}
            title="Close"
            style={{
              width: isMobile ? 32 : 18, height: isMobile ? 32 : 18, padding: 0,
              background: 'transparent', border: 'none',
              color: hw.textFaint, fontSize: isMobile ? 20 : 12, cursor: 'pointer',
            }}
          >×</button>
        )}
      </div>
      <div style={{ flex: 1, minHeight: 0, display: 'flex', overflow: 'hidden' }}>
        {children}
      </div>
      {!isMobile && (
        <div
          onMouseDown={startResize}
          title="Resize"
          style={{
            position: 'absolute',
            right: 0, bottom: 0,
            width: 14, height: 14,
            cursor: 'nwse-resize',
            background: `linear-gradient(135deg, transparent 50%, ${hw.border} 50%, ${hw.border} 60%, transparent 60%, transparent 75%, ${hw.border} 75%)`,
          }}
        />
      )}
    </div>,
    document.body,
  )
}

export function DetachButton({ panelId, title }: { panelId: PanelId; title?: string }) {
  // Pop the panel out into its OWN OS window (movable to another monitor) via
  // the open_panel_window command, then hide the inline copy in this window so
  // it isn't shown twice. The piano roll carries its open clip as context.
  const onClick = async () => {
    // Hide the inline copy right away — the panel is moving to its own window.
    window.dispatchEvent(new CustomEvent('daw:popoutPanel', { detail: panelId }))
    try {
      const { invoke } = await import('@tauri-apps/api/core')
      let params: string | undefined
      if (panelId === 'pianoRoll') {
        const s = useTrackStore.getState() as unknown as {
          activeMidiTrackId?: string | null; activeMidiClipId?: string | null
        }
        if (s.activeMidiTrackId && s.activeMidiClipId) {
          params = `trackId=${encodeURIComponent(s.activeMidiTrackId)}&clipId=${encodeURIComponent(s.activeMidiClipId)}`
        }
      }
      await invoke('open_panel_window', { panel: panelId, params })
    } catch (e) {
      console.warn('detach to window failed', e)
      // Window failed to open — bring the inline copy back.
      window.dispatchEvent(new CustomEvent('daw:redockPanel', { detail: panelId }))
    }
  }
  return (
    <button
      onClick={onClick}
      title={title || 'Pop out into its own window (drag to another monitor)'}
      style={{
        width: 18, height: 18, padding: 0,
        background: 'transparent', border: 'none',
        color: hw.textFaint, fontSize: 11, cursor: 'pointer',
      }}
    >⧉</button>
  )
}
