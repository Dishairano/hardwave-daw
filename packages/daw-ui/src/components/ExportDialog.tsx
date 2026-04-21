import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'

export type BitDepth = 0 | 16 | 24
export type SampleRate = 44100 | 48000 | 88200 | 96000 | 192000

export interface ExportSettings {
  bitDepth: BitDepth
  sampleRate: SampleRate
  tailSecs: number
  defaultName: string
}

interface Props {
  initial: ExportSettings
  onCancel: () => void
  onComplete: (result: { path: string; duration_secs: number }) => void
  onError: (message: string) => void
}

const BIT_DEPTH_OPTIONS: { value: BitDepth; label: string }[] = [
  { value: 0, label: '32-bit float' },
  { value: 24, label: '24-bit PCM' },
  { value: 16, label: '16-bit PCM' },
]

const SAMPLE_RATE_OPTIONS: SampleRate[] = [44100, 48000, 88200, 96000, 192000]

const LAST_EXPORT_DIR_KEY = 'hw.export.lastDir'
const BIT_DEPTH_KEY = 'hw.export.bitDepth'
const SAMPLE_RATE_KEY = 'hw.export.sampleRate'
const TAIL_SECS_KEY = 'hw.export.tailSecs'

function readNumber(key: string, fallback: number): number {
  try {
    const raw = localStorage.getItem(key)
    if (raw === null) return fallback
    const n = Number(raw)
    return Number.isFinite(n) ? n : fallback
  } catch { return fallback }
}

export function ExportDialog({ initial, onCancel, onComplete, onError }: Props) {
  const [bitDepth, setBitDepth] = useState<BitDepth>(() => readNumber(BIT_DEPTH_KEY, initial.bitDepth) as BitDepth)
  const [sampleRate, setSampleRate] = useState<SampleRate>(() => readNumber(SAMPLE_RATE_KEY, initial.sampleRate) as SampleRate)
  const [tailSecs, setTailSecs] = useState(() => readNumber(TAIL_SECS_KEY, initial.tailSecs))
  const [exporting, setExporting] = useState(false)
  const [percent, setPercent] = useState(0)
  const startBtn = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    startBtn.current?.focus()
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !exporting) onCancel()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onCancel, exporting])

  useEffect(() => {
    if (!exporting) return
    let cancelled = false
    const unlisten = listen<{ percent: number }>('export-progress', e => {
      if (!cancelled) setPercent(Math.max(0, Math.min(100, e.payload.percent)))
    })
    return () => {
      cancelled = true
      unlisten.then(fn => fn()).catch(() => {})
    }
  }, [exporting])

  const handleStart = async () => {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const lastDir = localStorage.getItem(LAST_EXPORT_DIR_KEY) || undefined
    const defaultPath = lastDir
      ? `${lastDir}/${initial.defaultName}.wav`
      : `${initial.defaultName}.wav`
    const path = await save({
      title: 'Export audio',
      defaultPath,
      filters: [{ name: 'WAV audio', extensions: ['wav'] }],
    })
    if (!path) return
    const sep = path.includes('\\') ? '\\' : '/'
    const idx = path.lastIndexOf(sep)
    if (idx > 0) localStorage.setItem(LAST_EXPORT_DIR_KEY, path.slice(0, idx))
    try { localStorage.setItem(BIT_DEPTH_KEY, String(bitDepth)) } catch {}
    try { localStorage.setItem(SAMPLE_RATE_KEY, String(sampleRate)) } catch {}
    try { localStorage.setItem(TAIL_SECS_KEY, String(tailSecs)) } catch {}

    setExporting(true)
    setPercent(0)
    try {
      const result = await invoke<{ path: string; duration_secs: number }>(
        'export_project_wav',
        { path, sampleRate, bitDepth, tailSecs },
      )
      onComplete(result)
    } catch (err) {
      onError(String(err))
    } finally {
      setExporting(false)
    }
  }

  return createPortal(
    <div style={{
      position: 'fixed', inset: 0, zIndex: 10000,
      background: 'rgba(0,0,0,0.6)',
      display: 'flex', alignItems: 'center', justifyContent: 'center',
    }}>
      <div style={{
        background: hw.bg,
        border: `1px solid ${hw.border}`,
        borderRadius: hw.radius.lg,
        padding: 20,
        width: 460,
        maxWidth: '90vw',
        boxShadow: '0 10px 40px rgba(0,0,0,0.6)',
      }}>
        <div style={{ fontSize: 15, fontWeight: 600, color: hw.textPrimary, marginBottom: 4 }}>
          Export audio
        </div>
        <div style={{ fontSize: 12, color: hw.textMuted, marginBottom: 16 }}>
          Render the full project to a WAV file.
        </div>

        <Row label="Format">
          <div style={{ fontSize: 12, color: hw.textSecondary }}>WAV</div>
        </Row>

        <Row label="Bit depth">
          <select
            value={bitDepth}
            disabled={exporting}
            onChange={e => setBitDepth(Number(e.target.value) as BitDepth)}
            style={selectStyle}
          >
            {BIT_DEPTH_OPTIONS.map(o => (
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
        </Row>

        <Row label="Sample rate">
          <select
            value={sampleRate}
            disabled={exporting}
            onChange={e => setSampleRate(Number(e.target.value) as SampleRate)}
            style={selectStyle}
          >
            {SAMPLE_RATE_OPTIONS.map(sr => (
              <option key={sr} value={sr}>{sr.toLocaleString()} Hz</option>
            ))}
          </select>
        </Row>

        <Row label={`Tail (${tailSecs.toFixed(1)}s)`}>
          <input
            type="range"
            min={0}
            max={30}
            step={0.5}
            value={tailSecs}
            disabled={exporting}
            onChange={e => setTailSecs(Number(e.target.value))}
            style={{ flex: 1 }}
          />
        </Row>

        {exporting && (
          <div style={{ marginTop: 16 }}>
            <div style={{
              fontSize: 11, color: hw.textMuted, marginBottom: 6,
              display: 'flex', justifyContent: 'space-between',
            }}>
              <span>Rendering…</span>
              <span>{percent.toFixed(0)}%</span>
            </div>
            <div style={{
              height: 6,
              background: hw.bgInput,
              borderRadius: 3,
              overflow: 'hidden',
              border: `1px solid ${hw.border}`,
            }}>
              <div style={{
                width: `${percent}%`,
                height: '100%',
                background: hw.accent,
                transition: 'width 0.12s linear',
              }} />
            </div>
          </div>
        )}

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 20 }}>
          <button
            onClick={onCancel}
            disabled={exporting}
            style={{
              ...btnStyle,
              opacity: exporting ? 0.5 : 1,
              cursor: exporting ? 'not-allowed' : 'pointer',
            }}
          >
            Cancel
          </button>
          <button
            ref={startBtn}
            onClick={handleStart}
            disabled={exporting}
            style={{
              ...btnStyle,
              background: exporting ? hw.bgElevated : hw.accent,
              color: exporting ? hw.textMuted : '#fff',
              border: `1px solid ${exporting ? hw.border : hw.accent}`,
              cursor: exporting ? 'not-allowed' : 'pointer',
            }}
          >
            {exporting ? 'Rendering…' : 'Start export'}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  )
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{
      display: 'flex', alignItems: 'center', gap: 12,
      padding: '8px 0',
    }}>
      <div style={{ fontSize: 12, color: hw.textMuted, width: 120 }}>{label}</div>
      <div style={{ flex: 1, display: 'flex', alignItems: 'center' }}>{children}</div>
    </div>
  )
}

const selectStyle: React.CSSProperties = {
  flex: 1,
  padding: '6px 8px',
  fontSize: 12,
  color: hw.textPrimary,
  background: hw.bgInput,
  border: `1px solid ${hw.border}`,
  borderRadius: hw.radius.sm,
  outline: 'none',
}

const btnStyle: React.CSSProperties = {
  padding: '8px 16px',
  fontSize: 13,
  color: hw.textPrimary,
  background: hw.bgElevated,
  border: `1px solid ${hw.border}`,
  borderRadius: hw.radius.md,
  minWidth: 110,
}
