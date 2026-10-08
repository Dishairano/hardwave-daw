import { useEffect, useRef, useState } from 'react'
import { useGeneralPrefsStore } from '../stores/generalPrefsStore'
import { usePerfMetersStore } from '../stores/perfMetersStore'
import { startFrameStats, type FrameSnapshot } from '../services/frameStats'

/**
 * Debug overlay: frames per second and what the interface is busy with.
 * Tools > Show FPS meter, or Ctrl+Shift+F. It ignores the mouse, so it
 * never gets in the way of what is under it.
 */
export function FpsMeter() {
  const on = useGeneralPrefsStore((s) => s.showFpsMeter)
  return on ? <FpsMeterPanel /> : null
}

const GRAPH_W = 196
const GRAPH_H = 36
/** The graph's top: frames this long or longer fill it. */
const GRAPH_MAX_MS = 66.7

function FpsMeterPanel() {
  const [snap, setSnap] = useState<FrameSnapshot | null>(null)
  const audioLoad = usePerfMetersStore((s) => s.cpuPct)
  const graphRef = useRef<HTMLCanvasElement>(null)

  useEffect(() => startFrameStats((s) => {
    setSnap(s)
    drawGraph(graphRef.current, s.recent)
  }), [])

  const fps = snap ? Math.round(snap.fps) : 0
  const fpsColor = !snap ? '#8a8a92' : fps >= 50 ? '#3ed07a' : fps >= 30 ? '#f0a032' : '#ff2d4f'

  return (
    <div data-testid="fps-meter" style={PANEL}>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 6 }}>
        <span style={{ fontSize: 20, fontWeight: 700, color: fpsColor }}>{snap ? fps : '--'}</span>
        <span style={{ color: '#8a8a92' }}>FPS</span>
        <span style={{ marginLeft: 'auto', color: '#8a8a92' }}>
          worst {snap ? snap.worstMs.toFixed(0) : '--'} ms
        </span>
      </div>
      <canvas ref={graphRef} width={GRAPH_W} height={GRAPH_H} style={{ width: GRAPH_W, height: GRAPH_H, display: 'block', margin: '4px 0' }} />
      <Row label="Slow frames" value={snap ? `${snap.slowPerSec.toFixed(0)}/s` : '--'} warn={!!snap && snap.slowPerSec >= 1} />
      <Row
        label="Long tasks"
        value={snap ? `${snap.longTasksPerSec.toFixed(1)}/s · ${snap.longTaskMs.toFixed(0)} ms` : '--'}
        warn={!!snap && snap.longTasksPerSec > 0}
      />
      <Row label="Renders" value={snap ? `${snap.rendersPerSec.toFixed(0)}/s` : '--'} warn={!!snap && snap.rendersPerSec > 40} />
      <Row label="Engine events" value={snap ? `${snap.engineEventsPerSec.toFixed(0)}/s` : '--'} />
      <Row
        label="Backend calls"
        value={snap ? `${snap.ipcPerSec.toFixed(0)}/s · ${snap.ipcAvgMs.toFixed(1)} ms` : '--'}
        warn={!!snap && snap.ipcAvgMs > 8}
      />
      <Row
        label="Slowest call"
        value={snap && snap.ipcSlowest ? `${snap.ipcMaxMs.toFixed(0)} ms` : '--'}
        warn={!!snap && snap.ipcMaxMs > 33}
      />
      {snap?.ipcSlowest && <div style={{ color: '#8a8a92', textAlign: 'right', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{snap.ipcSlowest}</div>}
      <Row label="Audio load" value={`${audioLoad}%`} warn={audioLoad >= 70} />
      {snap?.heapMb != null && <Row label="JS heap" value={`${snap.heapMb.toFixed(0)} MB`} />}
    </div>
  )
}

function Row({ label, value, warn }: { label: string; value: string; warn?: boolean }) {
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8 }}>
      <span style={{ color: '#8a8a92' }}>{label}</span>
      <span style={{ color: warn ? '#f0a032' : '#e8e8ec' }}>{value}</span>
    </div>
  )
}

/** Frame times as bars, newest on the right, with lines at 60 and 30 FPS. */
function drawGraph(canvas: HTMLCanvasElement | null, frames: number[]) {
  const ctx = canvas?.getContext('2d')
  if (!canvas || !ctx) return
  const w = canvas.width
  const h = canvas.height
  ctx.clearRect(0, 0, w, h)
  ctx.fillStyle = '#0a0a0c'
  ctx.fillRect(0, 0, w, h)
  const y = (ms: number) => h - Math.min(1, ms / GRAPH_MAX_MS) * h
  const barW = w / 120
  frames.forEach((ms, i) => {
    ctx.fillStyle = ms > 33.4 ? '#ff2d4f' : ms > 17.5 ? '#f0a032' : '#3ed07a'
    const top = y(ms)
    ctx.fillRect(w - (frames.length - i) * barW, top, Math.max(1, barW - 0.5), h - top)
  })
  ctx.fillStyle = 'rgba(255,255,255,0.25)'
  ctx.fillRect(0, Math.round(y(16.7)), w, 1)
  ctx.fillRect(0, Math.round(y(33.3)), w, 1)
}

const PANEL: React.CSSProperties = {
  position: 'fixed',
  top: 96,
  right: 12,
  zIndex: 100000,
  width: GRAPH_W + 16,
  padding: 8,
  borderRadius: 6,
  background: 'rgba(10,10,12,0.92)',
  border: '1px solid #2a2a32',
  color: '#e8e8ec',
  font: '11px/1.5 "JetBrains Mono", ui-monospace, monospace',
  pointerEvents: 'none',
  userSelect: 'none',
}
