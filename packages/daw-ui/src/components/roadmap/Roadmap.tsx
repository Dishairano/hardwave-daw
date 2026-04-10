import { useState, useRef, useCallback, useEffect } from 'react'
import { hw } from '../../theme'

const ROADMAP_URL = 'https://suite.hardwavestudios.com/roadmap?embedded=1'
const DEFAULT_W = 540
const DEFAULT_H = 600

interface RoadmapProps {
  onClose: () => void
}

export function Roadmap({ onClose }: RoadmapProps) {
  const [pos, setPos] = useState({ x: Math.round((window.innerWidth - DEFAULT_W) / 2), y: 50 })
  const [size, setSize] = useState({ w: DEFAULT_W, h: DEFAULT_H })

  const dragRef = useRef<{ startX: number; startY: number; origX: number; origY: number } | null>(null)
  const resizeRef = useRef<{ startX: number; startY: number; origW: number; origH: number } | null>(null)
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const [dragging, setDragging] = useState(false)

  const onTitleMouseDown = useCallback((e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest('[data-no-drag]')) return
    e.preventDefault()
    dragRef.current = { startX: e.clientX, startY: e.clientY, origX: pos.x, origY: pos.y }
    setDragging(true)
  }, [pos])

  const onResizeMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    resizeRef.current = { startX: e.clientX, startY: e.clientY, origW: size.w, origH: size.h }
    setDragging(true)
  }, [size])

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (dragRef.current) {
        setPos({
          x: dragRef.current.origX + e.clientX - dragRef.current.startX,
          y: Math.max(0, dragRef.current.origY + e.clientY - dragRef.current.startY),
        })
      }
      if (resizeRef.current) {
        setSize({
          w: Math.max(360, resizeRef.current.origW + e.clientX - resizeRef.current.startX),
          h: Math.max(300, resizeRef.current.origH + e.clientY - resizeRef.current.startY),
        })
      }
    }
    const onUp = () => {
      dragRef.current = null
      resizeRef.current = null
      setDragging(false)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
  }, [])

  return (
    <div style={{
      position: 'fixed',
      left: pos.x, top: pos.y,
      width: size.w, height: size.h,
      zIndex: 80,
      display: 'flex', flexDirection: 'column',
      background: 'rgba(12,12,16,0.97)',
      backdropFilter: 'blur(16px)',
      border: `1px solid ${hw.borderLight}`,
      borderRadius: hw.radius.lg,
      boxShadow: '0 20px 60px rgba(0,0,0,0.7), 0 0 1px rgba(255,255,255,0.06)',
      overflow: 'hidden',
    }}>
      {/* Title bar */}
      <div
        onMouseDown={onTitleMouseDown}
        style={{
          display: 'flex', alignItems: 'center',
          height: 32, padding: '0 10px',
          background: 'rgba(255,255,255,0.03)',
          borderBottom: `1px solid ${hw.border}`,
          cursor: 'grab', flexShrink: 0, userSelect: 'none',
        }}
      >
        <span style={{ fontSize: 11, fontWeight: 700, color: hw.accent, letterSpacing: 0.5 }}>
          ROADMAP
        </span>
        <a
          data-no-drag
          href="https://suite.hardwavestudios.com/roadmap"
          target="_blank"
          rel="noopener noreferrer"
          style={{
            fontSize: 9, color: hw.textFaint, marginLeft: 8,
            textDecoration: 'none',
          }}
          onMouseEnter={e => { e.currentTarget.style.color = hw.textMuted }}
          onMouseLeave={e => { e.currentTarget.style.color = hw.textFaint }}
        >
          Open in browser
        </a>
        <div style={{ flex: 1 }} />
        <div
          data-no-drag
          onClick={onClose}
          style={{
            width: 20, height: 20,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            borderRadius: 4, color: hw.textFaint, cursor: 'pointer',
          }}
          onMouseEnter={e => {
            e.currentTarget.style.background = 'rgba(255,255,255,0.08)'
            e.currentTarget.style.color = hw.textPrimary
          }}
          onMouseLeave={e => {
            e.currentTarget.style.background = 'transparent'
            e.currentTarget.style.color = hw.textFaint
          }}
        >
          <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <line x1="2" y1="2" x2="10" y2="10" /><line x1="10" y1="2" x2="2" y2="10" />
          </svg>
        </div>
      </div>

      {/* Iframe */}
      <iframe
        ref={iframeRef}
        src={ROADMAP_URL}
        style={{
          flex: 1, border: 'none', background: 'transparent',
          pointerEvents: dragging ? 'none' : 'auto',
        }}
      />

      {/* Resize handle */}
      <div
        onMouseDown={onResizeMouseDown}
        style={{ position: 'absolute', right: 0, bottom: 0, width: 16, height: 16, cursor: 'nwse-resize' }}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" style={{ position: 'absolute', right: 3, bottom: 3 }}>
          <line x1="8" y1="1" x2="1" y2="8" stroke={hw.textFaint} strokeWidth="1" strokeOpacity="0.3" />
          <line x1="8" y1="4" x2="4" y2="8" stroke={hw.textFaint} strokeWidth="1" strokeOpacity="0.3" />
          <line x1="8" y1="7" x2="7" y2="8" stroke={hw.textFaint} strokeWidth="1" strokeOpacity="0.3" />
        </svg>
      </div>
    </div>
  )
}
