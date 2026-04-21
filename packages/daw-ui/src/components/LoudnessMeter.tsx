import { useEffect, useRef, useState } from 'react'
import { hw } from '../theme'
import { useMeterStore } from '../stores/meterStore'

interface LoudnessMeterProps {
  onClose: () => void
}

const HISTORY_SECONDS = 30
const SAMPLES_PER_SEC = 10
const HISTORY_LEN = HISTORY_SECONDS * SAMPLES_PER_SEC

function formatLufs(v: number | null | undefined) {
  if (v === null || v === undefined || !isFinite(v)) return '—'
  return v.toFixed(1)
}

export function LoudnessMeter({ onClose }: LoudnessMeterProps) {
  const master = useMeterStore(s => s.master)
  const [target, setTarget] = useState(-14)
  const [history, setHistory] = useState<{ m: number | null; s: number | null; i: number | null }[]>(
    () => Array.from({ length: HISTORY_LEN }, () => ({ m: null, s: null, i: null }))
  )
  const canvasRef = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const id = setInterval(() => {
      setHistory(prev => {
        const { master: m } = useMeterStore.getState()
        const next = prev.slice(1)
        next.push({ m: m.lufs_m ?? null, s: m.lufs_s ?? null, i: m.lufs_i ?? null })
        return next
      })
    }, 1000 / SAMPLES_PER_SEC)
    return () => clearInterval(id)
  }, [])

  useEffect(() => {
    const c = canvasRef.current
    if (!c) return
    draw(c, history, target)
  }, [history, target])

  const reset = () => {
    setHistory(Array.from({ length: HISTORY_LEN }, () => ({ m: null, s: null, i: null })))
  }

  const [copied, setCopied] = useState(false)
  const copyReadings = async () => {
    const rows = [
      `Momentary:  ${formatLufs(master.lufs_m)} LUFS`,
      `Short-term: ${formatLufs(master.lufs_s)} LUFS`,
      `Integrated: ${formatLufs(master.lufs_i)} LUFS`,
      `True Peak:  ${isFinite(master.true_peak_db) ? master.true_peak_db.toFixed(1) : '—'} dBTP`,
      `Target:     ${target.toFixed(1)} LUFS`,
    ]
    try {
      await navigator.clipboard.writeText(rows.join('\n'))
      setCopied(true)
      setTimeout(() => setCopied(false), 1200)
    } catch { /* ignore */ }
  }

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 9800,
        background: 'rgba(0,0,0,0.45)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div
        style={{
          width: 560, maxWidth: '92vw',
          background: hw.bg, color: hw.textPrimary,
          border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg,
          overflow: 'hidden',
        }}
      >
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center',
          gap: 12, background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Loudness Meter</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>BS.1770 · 30s history</div>
          <div style={{ flex: 1 }} />
          <button onClick={copyReadings} style={{
            padding: '3px 10px', fontSize: 10, background: 'transparent',
            border: `1px solid ${copied ? hw.accent : hw.border}`, borderRadius: hw.radius.sm,
            color: copied ? hw.accent : hw.textSecondary, cursor: 'pointer',
            transition: 'color 0.15s, border-color 0.15s',
          }}>
            {copied ? 'Copied!' : 'Copy readings'}
          </button>
          <button onClick={reset} style={{
            padding: '3px 10px', fontSize: 10, background: 'transparent',
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
            color: hw.textSecondary, cursor: 'pointer',
          }}>
            Clear history
          </button>
          <button onClick={onClose} style={{
            padding: '3px 10px', fontSize: 10, background: 'transparent',
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
            color: hw.textSecondary, cursor: 'pointer',
          }}>
            Close
          </button>
        </div>

        <div style={{ padding: 12, display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 8 }}>
          <Reading label="Momentary" unit="LUFS" value={formatLufs(master.lufs_m)} target={target} dbValue={master.lufs_m} />
          <Reading label="Short-term" unit="LUFS" value={formatLufs(master.lufs_s)} target={target} dbValue={master.lufs_s} />
          <Reading label="Integrated" unit="LUFS" value={formatLufs(master.lufs_i)} target={target} dbValue={master.lufs_i} />
          <Reading label="True Peak" unit="dBTP"
            value={isFinite(master.true_peak_db) ? master.true_peak_db.toFixed(1) : '—'}
            target={0} dbValue={master.true_peak_db} invertTolerance />
        </div>

        <div style={{
          padding: '8px 12px', borderTop: `1px solid ${hw.border}`,
          background: hw.bgElevated, display: 'flex', alignItems: 'center', gap: 12,
        }}>
          <label style={{ fontSize: 10, color: hw.textSecondary, display: 'flex', gap: 8, alignItems: 'center' }}>
            <span>Target</span>
            <input type="range" min={-30} max={-6} step={0.5} value={target}
              onChange={(e) => setTarget(parseFloat(e.target.value))}
              style={{ width: 140 }} />
            <span style={{ color: hw.textPrimary, fontVariantNumeric: 'tabular-nums', minWidth: 48 }}>
              {target.toFixed(1)} LUFS
            </span>
          </label>
          <div style={{ flex: 1 }} />
          <div style={{ fontSize: 9, color: hw.textFaint }}>
            <span style={{ color: hw.accent }}>━</span> Momentary &nbsp;
            <span style={{ color: hw.secondary }}>━</span> Short-term &nbsp;
            <span style={{ color: hw.textMuted }}>━</span> Integrated &nbsp;
            <span style={{ color: hw.textFaint }}>┄</span> Target
          </div>
        </div>

        <canvas
          ref={canvasRef}
          width={560}
          height={170}
          style={{ width: '100%', height: 170, background: '#0a0a0e', display: 'block' }}
        />
      </div>
    </div>
  )
}

function Reading({
  label, unit, value, target, dbValue, invertTolerance,
}: {
  label: string; unit: string; value: string; target: number;
  dbValue: number | null | undefined; invertTolerance?: boolean;
}) {
  let color = hw.textPrimary
  if (dbValue !== null && dbValue !== undefined && isFinite(dbValue)) {
    const delta = dbValue - target
    const high = invertTolerance ? delta > -1 : delta > 1
    const low = invertTolerance ? delta < -6 : delta < -6
    if (high) color = hw.red
    else if (low) color = hw.textMuted
  }
  return (
    <div style={{
      padding: 10, background: hw.bgElevated,
      border: `1px solid ${hw.border}`, borderRadius: hw.radius.md, textAlign: 'center',
    }}>
      <div style={{ fontSize: 9, color: hw.textFaint, textTransform: 'uppercase', letterSpacing: 0.5 }}>
        {label}
      </div>
      <div style={{ fontSize: 20, fontWeight: 700, color, fontVariantNumeric: 'tabular-nums', marginTop: 4 }}>
        {value}
      </div>
      <div style={{ fontSize: 8, color: hw.textFaint, marginTop: 2 }}>{unit}</div>
    </div>
  )
}

function draw(
  c: HTMLCanvasElement,
  history: { m: number | null; s: number | null; i: number | null }[],
  target: number,
) {
  const ctx = c.getContext('2d')
  if (!ctx) return
  const dpr = window.devicePixelRatio || 1
  const w = c.clientWidth
  const h = c.clientHeight
  if (c.width !== Math.floor(w * dpr) || c.height !== Math.floor(h * dpr)) {
    c.width = Math.floor(w * dpr)
    c.height = Math.floor(h * dpr)
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx.fillStyle = '#0a0a0e'
  ctx.fillRect(0, 0, w, h)

  const minDb = -60
  const maxDb = 0
  const y = (db: number) => {
    const clamped = Math.max(minDb, Math.min(maxDb, db))
    return h - ((clamped - minDb) / (maxDb - minDb)) * h
  }

  ctx.strokeStyle = hw.border
  ctx.lineWidth = 1
  ctx.setLineDash([])
  for (let db = -60; db <= 0; db += 10) {
    ctx.beginPath()
    const py = y(db)
    ctx.moveTo(0, py)
    ctx.lineTo(w, py)
    ctx.stroke()
    ctx.fillStyle = hw.textFaint
    ctx.font = '9px sans-serif'
    ctx.fillText(`${db}`, 2, py - 2)
  }

  ctx.strokeStyle = hw.textFaint
  ctx.setLineDash([4, 3])
  ctx.beginPath()
  const ty = y(target)
  ctx.moveTo(0, ty)
  ctx.lineTo(w, ty)
  ctx.stroke()
  ctx.setLineDash([])

  const plot = (color: string, key: 'm' | 's' | 'i', width: number) => {
    ctx.strokeStyle = color
    ctx.lineWidth = width
    ctx.beginPath()
    let started = false
    for (let i = 0; i < history.length; i++) {
      const v = history[i][key]
      if (v === null || !isFinite(v)) { started = false; continue }
      const px = (i / (history.length - 1)) * w
      const py = y(v)
      if (!started) { ctx.moveTo(px, py); started = true } else ctx.lineTo(px, py)
    }
    ctx.stroke()
  }
  plot(hw.textMuted, 'i', 2)
  plot(hw.secondary, 's', 1.5)
  plot(hw.accent, 'm', 1)
}
