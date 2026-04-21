import { useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useTransportStore } from '../stores/transportStore'

export type BitDepth = 0 | 16 | 24
export type SampleRate = 44100 | 48000 | 88200 | 96000 | 192000
export type ExportRange = 'full' | 'loop'
export type NormalizeMode = 'off' | 'peak'
export type DitherMode = 'none' | 'tpdf' | 'tpdf_shaped'

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
const STEMS_KEY = 'hw.export.stems'
const STEMS_INCLUDE_MASTER_KEY = 'hw.export.stemsIncludeMaster'
const STEMS_RESPECT_MUTE_SOLO_KEY = 'hw.export.stemsRespectMuteSolo'
const RANGE_KEY = 'hw.export.range'
const PLAY_AFTER_KEY = 'hw.export.playAfter'
const NORMALIZE_KEY = 'hw.export.normalize'
const NORMALIZE_DB_KEY = 'hw.export.normalizeDb'
const DITHER_KEY = 'hw.export.dither'

function readNumber(key: string, fallback: number): number {
  try {
    const raw = localStorage.getItem(key)
    if (raw === null) return fallback
    const n = Number(raw)
    return Number.isFinite(n) ? n : fallback
  } catch { return fallback }
}

function readBool(key: string, fallback: boolean): boolean {
  try {
    const raw = localStorage.getItem(key)
    if (raw === null) return fallback
    return raw === '1' || raw === 'true'
  } catch { return fallback }
}

export function ExportDialog({ initial, onCancel, onComplete, onError }: Props) {
  const [bitDepth, setBitDepth] = useState<BitDepth>(() => readNumber(BIT_DEPTH_KEY, initial.bitDepth) as BitDepth)
  const [sampleRate, setSampleRate] = useState<SampleRate>(() => readNumber(SAMPLE_RATE_KEY, initial.sampleRate) as SampleRate)
  const [tailSecs, setTailSecs] = useState(() => readNumber(TAIL_SECS_KEY, initial.tailSecs))
  const [stems, setStems] = useState<boolean>(() => readBool(STEMS_KEY, false))
  const [includeMaster, setIncludeMaster] = useState<boolean>(() => readBool(STEMS_INCLUDE_MASTER_KEY, true))
  const [respectMuteSolo, setRespectMuteSolo] = useState<boolean>(() => readBool(STEMS_RESPECT_MUTE_SOLO_KEY, false))
  const [range, setRange] = useState<ExportRange>(() => {
    const raw = (typeof localStorage !== 'undefined' && localStorage.getItem(RANGE_KEY)) || 'full'
    return raw === 'loop' ? 'loop' : 'full'
  })
  const [playAfter, setPlayAfter] = useState<boolean>(() => readBool(PLAY_AFTER_KEY, false))
  const [normalizeMode, setNormalizeMode] = useState<NormalizeMode>(() => {
    const raw = (typeof localStorage !== 'undefined' && localStorage.getItem(NORMALIZE_KEY)) || 'off'
    return raw === 'peak' ? 'peak' : 'off'
  })
  const [normalizeTargetDb, setNormalizeTargetDb] = useState<number>(() => readNumber(NORMALIZE_DB_KEY, -1.0))
  const [ditherMode, setDitherMode] = useState<DitherMode>(() => {
    const raw = (typeof localStorage !== 'undefined' && localStorage.getItem(DITHER_KEY)) || 'none'
    if (raw === 'tpdf') return 'tpdf'
    if (raw === 'tpdf_shaped') return 'tpdf_shaped'
    return 'none'
  })
  const [exporting, setExporting] = useState(false)
  const [percent, setPercent] = useState(0)
  const [stageLabel, setStageLabel] = useState<string | null>(null)
  const [etaSecs, setEtaSecs] = useState<number | null>(null)
  const renderStartedAt = useRef<number>(0)
  const startBtn = useRef<HTMLButtonElement>(null)

  const loopStart = useTransportStore(s => s.loopStart)
  const loopEnd = useTransportStore(s => s.loopEnd)
  const projectSampleRate = useTransportStore(s => s.sampleRate)
  const loopRangeValid = loopEnd > loopStart

  const loopDurationSecs = useMemo(() => {
    if (!loopRangeValid || projectSampleRate <= 0) return 0
    return (loopEnd - loopStart) / projectSampleRate
  }, [loopStart, loopEnd, projectSampleRate, loopRangeValid])

  useEffect(() => {
    if (range === 'loop' && !loopRangeValid) setRange('full')
  }, [range, loopRangeValid])

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
    const unlisten = listen<{ percent: number; label?: string | null }>('export-progress', e => {
      if (cancelled) return
      const pct = Math.max(0, Math.min(100, e.payload.percent))
      setPercent(pct)
      if (e.payload.label !== undefined) setStageLabel(e.payload.label ?? null)
      if (pct > 0.5 && renderStartedAt.current > 0) {
        const elapsed = (performance.now() - renderStartedAt.current) / 1000
        const total = elapsed * (100 / pct)
        setEtaSecs(Math.max(0, total - elapsed))
      }
    })
    return () => {
      cancelled = true
      unlisten.then(fn => fn()).catch(() => {})
    }
  }, [exporting])

  const persistPrefs = () => {
    try { localStorage.setItem(BIT_DEPTH_KEY, String(bitDepth)) } catch {}
    try { localStorage.setItem(SAMPLE_RATE_KEY, String(sampleRate)) } catch {}
    try { localStorage.setItem(TAIL_SECS_KEY, String(tailSecs)) } catch {}
    try { localStorage.setItem(STEMS_KEY, stems ? '1' : '0') } catch {}
    try { localStorage.setItem(STEMS_INCLUDE_MASTER_KEY, includeMaster ? '1' : '0') } catch {}
    try { localStorage.setItem(STEMS_RESPECT_MUTE_SOLO_KEY, respectMuteSolo ? '1' : '0') } catch {}
    try { localStorage.setItem(RANGE_KEY, range) } catch {}
    try { localStorage.setItem(PLAY_AFTER_KEY, playAfter ? '1' : '0') } catch {}
    try { localStorage.setItem(NORMALIZE_KEY, normalizeMode) } catch {}
    try { localStorage.setItem(NORMALIZE_DB_KEY, String(normalizeTargetDb)) } catch {}
    try { localStorage.setItem(DITHER_KEY, ditherMode) } catch {}
  }

  const computeRenderBounds = (): { startSamples: number | null; endSamples: number | null } => {
    if (range !== 'loop' || !loopRangeValid || projectSampleRate <= 0 || projectSampleRate === sampleRate) {
      if (range === 'loop' && loopRangeValid) {
        return { startSamples: loopStart, endSamples: loopEnd }
      }
      return { startSamples: null, endSamples: null }
    }
    const ratio = sampleRate / projectSampleRate
    return {
      startSamples: Math.round(loopStart * ratio),
      endSamples: Math.round(loopEnd * ratio),
    }
  }

  const handleStart = async () => {
    const { save, open } = await import('@tauri-apps/plugin-dialog')
    const lastDir = localStorage.getItem(LAST_EXPORT_DIR_KEY) || undefined

    if (stems) {
      const folder = await open({
        title: 'Export stems to folder',
        directory: true,
        defaultPath: lastDir,
      })
      if (!folder) return
      const folderPath = Array.isArray(folder) ? folder[0] : folder
      localStorage.setItem(LAST_EXPORT_DIR_KEY, folderPath)
      persistPrefs()

      setExporting(true)
      setPercent(0)
      setStageLabel(null)
      setEtaSecs(null)
      renderStartedAt.current = performance.now()
      const stemsBounds = computeRenderBounds()
      try {
        const result = await invoke<{
          folder: string
          files: string[]
          duration_secs: number
          cancelled: boolean
        }>('export_project_stems', {
          folderPath,
          projectName: initial.defaultName,
          sampleRate,
          bitDepth,
          tailSecs,
          includeMaster,
          respectMuteSolo,
          startSamples: stemsBounds.startSamples,
          endSamples: stemsBounds.endSamples,
          normalizeMode,
          normalizeTargetDb,
          ditherMode,
        })
        if (result.cancelled) {
          onError('Export cancelled')
        } else {
          onComplete({ path: result.folder, duration_secs: result.duration_secs })
        }
      } catch (err) {
        onError(String(err))
      } finally {
        setExporting(false)
      }
      return
    }

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
    persistPrefs()

    setExporting(true)
    setPercent(0)
    setStageLabel(null)
    setEtaSecs(null)
    renderStartedAt.current = performance.now()
    const { startSamples, endSamples } = computeRenderBounds()
    try {
      const result = await invoke<{
        path: string
        duration_secs: number
        cancelled: boolean
      }>('export_project_wav', {
        path,
        sampleRate,
        bitDepth,
        tailSecs,
        startSamples,
        endSamples,
        normalizeMode,
        normalizeTargetDb,
        ditherMode,
      })
      if (result.cancelled) {
        onError('Export cancelled')
      } else {
        if (playAfter) {
          try {
            const { openPath } = await import('@tauri-apps/plugin-opener')
            await openPath(result.path)
          } catch {}
        }
        onComplete(result)
      }
    } catch (err) {
      onError(String(err))
    } finally {
      setExporting(false)
    }
  }

  const handleCancelRender = async () => {
    try { await invoke('cancel_export') } catch {}
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

        <Row label="Range">
          <select
            value={range}
            disabled={exporting}
            onChange={e => setRange(e.target.value as ExportRange)}
            style={selectStyle}
          >
            <option value="full">Full project</option>
            <option value="loop" disabled={!loopRangeValid}>
              {loopRangeValid
                ? `Loop region (${loopDurationSecs.toFixed(2)}s)`
                : 'Loop region (no range set)'}
            </option>
          </select>
        </Row>

        <Row label="Normalize">
          <select
            value={normalizeMode}
            disabled={exporting}
            onChange={e => setNormalizeMode(e.target.value as NormalizeMode)}
            style={selectStyle}
          >
            <option value="off">Off</option>
            <option value="peak">Peak</option>
          </select>
        </Row>
        {normalizeMode === 'peak' && (
          <Row label={`Target (${normalizeTargetDb.toFixed(1)} dB)`}>
            <input
              type="range"
              min={-12}
              max={0}
              step={0.1}
              value={normalizeTargetDb}
              disabled={exporting}
              onChange={e => setNormalizeTargetDb(Number(e.target.value))}
              style={{ flex: 1 }}
            />
          </Row>
        )}

        <Row label="Dither">
          <select
            value={ditherMode}
            disabled={exporting || bitDepth === 0}
            onChange={e => setDitherMode(e.target.value as DitherMode)}
            style={selectStyle}
          >
            <option value="none">None</option>
            <option value="tpdf">TPDF</option>
            <option value="tpdf_shaped">TPDF noise-shaped</option>
          </select>
        </Row>

        <Row label="After">
          <label style={checkLabel}>
            <input
              type="checkbox"
              checked={playAfter}
              disabled={exporting}
              onChange={e => setPlayAfter(e.target.checked)}
            />
            <span>Play rendered file when done</span>
          </label>
        </Row>

        <Row label="Stems">
          <label style={checkLabel}>
            <input
              type="checkbox"
              checked={stems}
              disabled={exporting}
              onChange={e => setStems(e.target.checked)}
            />
            <span>Export each track to a separate file</span>
          </label>
        </Row>
        {stems && (
          <>
            <Row label="">
              <label style={checkLabel}>
                <input
                  type="checkbox"
                  checked={includeMaster}
                  disabled={exporting}
                  onChange={e => setIncludeMaster(e.target.checked)}
                />
                <span>Include master bus stem</span>
              </label>
            </Row>
            <Row label="">
              <label style={checkLabel}>
                <input
                  type="checkbox"
                  checked={respectMuteSolo}
                  disabled={exporting}
                  onChange={e => setRespectMuteSolo(e.target.checked)}
                />
                <span>Respect existing mute / solo state</span>
              </label>
            </Row>
          </>
        )}

        {exporting && (
          <div style={{ marginTop: 16 }}>
            <div style={{
              fontSize: 11, color: hw.textMuted, marginBottom: 6,
              display: 'flex', justifyContent: 'space-between',
            }}>
              <span>{stageLabel ? `Rendering: ${stageLabel}` : 'Rendering…'}</span>
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
            <div style={{
              fontSize: 10, color: hw.textMuted, marginTop: 6,
              display: 'flex', justifyContent: 'flex-end',
            }}>
              {etaSecs !== null && percent < 100
                ? `~${formatEta(etaSecs)} remaining`
                : ' '}
            </div>
          </div>
        )}

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 20 }}>
          {exporting ? (
            <button
              onClick={handleCancelRender}
              style={{
                ...btnStyle,
                color: hw.red,
                borderColor: hw.red,
              }}
            >
              Cancel render
            </button>
          ) : (
            <>
              <button onClick={onCancel} style={btnStyle}>Cancel</button>
              <button
                ref={startBtn}
                onClick={handleStart}
                style={{
                  ...btnStyle,
                  background: hw.accent,
                  color: '#fff',
                  border: `1px solid ${hw.accent}`,
                }}
              >
                {stems ? 'Export stems' : 'Start export'}
              </button>
            </>
          )}
        </div>
      </div>
    </div>,
    document.body,
  )
}

function formatEta(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return '—'
  if (secs < 1) return '<1s'
  if (secs < 60) return `${Math.ceil(secs)}s`
  const mins = Math.floor(secs / 60)
  const rem = Math.round(secs - mins * 60)
  return rem === 0 ? `${mins}m` : `${mins}m ${rem}s`
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
  cursor: 'pointer',
}

const checkLabel: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  fontSize: 12,
  color: hw.textSecondary,
  cursor: 'pointer',
}
