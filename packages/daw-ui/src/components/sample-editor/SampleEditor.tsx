import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { hw } from '../../theme'
import { encodeWav } from '../../utils/wav'

interface SampleEditorProps {
  path: string
  onClose: () => void
}

interface Sample {
  channels: Float32Array[]
  sampleRate: number
}

interface Selection {
  start: number
  end: number
}

function emptySelection(): Selection { return { start: 0, end: 0 } }
function selLength(s: Selection) { return Math.abs(s.end - s.start) }
function normSel(s: Selection): Selection {
  return s.start <= s.end ? s : { start: s.end, end: s.start }
}

export function SampleEditor({ path, onClose }: SampleEditorProps) {
  const [sample, setSample] = useState<Sample | null>(null)
  const [loading, setLoading] = useState(true)
  const [err, setErr] = useState<string | null>(null)
  const [selection, setSelection] = useState<Selection>(emptySelection())
  const [view, setView] = useState<Selection>({ start: 0, end: 1 })
  const [playing, setPlaying] = useState(false)
  const [clipboard, setClipboard] = useState<Float32Array[] | null>(null)
  const [banner, setBanner] = useState<{ kind: 'info' | 'error'; text: string } | null>(null)
  const [undoStack, setUndoStack] = useState<Sample[]>([])

  const canvasRef = useRef<HTMLCanvasElement>(null)
  const playCtxRef = useRef<AudioContext | null>(null)
  const playSrcRef = useRef<AudioBufferSourceNode | null>(null)

  const name = path.split(/[\\/]/).pop() || path

  // ── Load file ──────────────────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false
    ;(async () => {
      setLoading(true)
      setErr(null)
      try {
        const { convertFileSrc } = await import('@tauri-apps/api/core')
        const url = convertFileSrc(path)
        const resp = await fetch(url)
        if (!resp.ok) throw new Error('Failed to fetch audio file')
        const buf = await resp.arrayBuffer()
        const AC: typeof AudioContext = (window as unknown as { AudioContext: typeof AudioContext; webkitAudioContext?: typeof AudioContext }).AudioContext
          || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext
        const ctx = new AC()
        const decoded = await ctx.decodeAudioData(buf.slice(0))
        if (cancelled) return
        const channels: Float32Array[] = []
        for (let i = 0; i < decoded.numberOfChannels; i++) {
          channels.push(new Float32Array(decoded.getChannelData(i)))
        }
        setSample({ channels, sampleRate: decoded.sampleRate })
        ctx.close().catch(() => {})
      } catch (e) {
        if (!cancelled) setErr(String(e))
      }
      if (!cancelled) setLoading(false)
    })()
    return () => { cancelled = true }
  }, [path])

  // ── Stop playback on close/unmount ─────────────────────────────────────────
  useEffect(() => {
    return () => {
      try { playSrcRef.current?.stop() } catch { /* ignore */ }
      playCtxRef.current?.close().catch(() => {})
    }
  }, [])

  // ── Redraw waveform ────────────────────────────────────────────────────────
  useEffect(() => {
    if (!sample || !canvasRef.current) return
    drawWaveform(canvasRef.current, sample, view, selection)
  }, [sample, view, selection])

  // ── Helpers ────────────────────────────────────────────────────────────────
  const pushUndo = useCallback((current: Sample) => {
    setUndoStack(prev => {
      const next = [...prev, cloneSample(current)]
      if (next.length > 20) next.shift()
      return next
    })
  }, [])

  const commit = useCallback((mutator: (s: Sample) => Sample, msg?: string) => {
    setSample(prev => {
      if (!prev) return prev
      pushUndo(prev)
      const next = mutator(prev)
      return next
    })
    if (msg) setBanner({ kind: 'info', text: msg })
  }, [pushUndo])

  const undo = () => {
    if (undoStack.length === 0) return
    setSample(undoStack[undoStack.length - 1])
    setUndoStack(prev => prev.slice(0, -1))
    setBanner({ kind: 'info', text: 'Undo' })
  }

  // Selection in normalized 0-1, convert to sample index
  const selIndices = useMemo((): [number, number] | null => {
    if (!sample) return null
    const n = sample.channels[0]?.length ?? 0
    if (n === 0) return null
    const s = normSel(selection)
    if (selLength(s) < 1e-6) return null
    return [Math.floor(s.start * n), Math.floor(s.end * n)]
  }, [selection, sample])

  const hasSelection = !!selIndices

  // ── Playback ───────────────────────────────────────────────────────────────
  const stopPlay = useCallback(() => {
    try { playSrcRef.current?.stop() } catch { /* ignore */ }
    playSrcRef.current = null
    playCtxRef.current?.close().catch(() => {})
    playCtxRef.current = null
    setPlaying(false)
  }, [])

  const play = () => {
    if (!sample) return
    stopPlay()
    const AC: typeof AudioContext = (window as unknown as { AudioContext: typeof AudioContext; webkitAudioContext?: typeof AudioContext }).AudioContext
      || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext
    const ctx = new AC({ sampleRate: sample.sampleRate })
    const len = sample.channels[0].length
    const buf = ctx.createBuffer(sample.channels.length, len, sample.sampleRate)
    for (let c = 0; c < sample.channels.length; c++) {
      buf.getChannelData(c).set(sample.channels[c])
    }
    const src = ctx.createBufferSource()
    src.buffer = buf
    src.connect(ctx.destination)
    src.onended = () => {
      setPlaying(false)
      playSrcRef.current = null
    }

    if (hasSelection && selIndices) {
      const [s, e] = selIndices
      const start = s / sample.sampleRate
      const dur = Math.max(0.01, (e - s) / sample.sampleRate)
      src.start(0, start, dur)
    } else {
      src.start()
    }
    playCtxRef.current = ctx
    playSrcRef.current = src
    setPlaying(true)
  }

  // ── Editing ops ────────────────────────────────────────────────────────────
  const selectAll = () => setSelection({ start: 0, end: 1 })
  const deselect = () => setSelection(emptySelection())

  const deleteSel = () => {
    if (!sample || !selIndices) return
    const [s, e] = selIndices
    commit(prev => ({
      ...prev,
      channels: prev.channels.map(ch => sliceRemove(ch, s, e)),
    }), `Deleted ${formatRange(e - s, sample.sampleRate)}`)
    setSelection(emptySelection())
  }

  const copySel = () => {
    if (!sample || !selIndices) return
    const [s, e] = selIndices
    setClipboard(sample.channels.map(ch => ch.slice(s, e)))
    setBanner({ kind: 'info', text: `Copied ${formatRange(e - s, sample.sampleRate)}` })
  }

  const cutSel = () => {
    if (!sample || !selIndices) return
    copySel()
    deleteSel()
  }

  const pasteAtCursor = () => {
    if (!sample || !clipboard) {
      setBanner({ kind: 'error', text: 'Clipboard is empty' })
      return
    }
    const pasteLen = clipboard[0].length
    const insertAt = selIndices ? selIndices[0] : 0
    commit(prev => ({
      ...prev,
      channels: prev.channels.map((ch, i) => {
        const paste = clipboard[i] ?? clipboard[0] // fall back to mono if channel count mismatches
        const out = new Float32Array(ch.length + paste.length)
        out.set(ch.subarray(0, insertAt), 0)
        out.set(paste, insertAt)
        out.set(ch.subarray(insertAt), insertAt + paste.length)
        return out
      }),
    }), `Pasted ${formatRange(pasteLen, sample.sampleRate)}`)
  }

  const normalizePeak = () => {
    if (!sample) return
    commit(prev => {
      const [s, e] = selIndices ?? [0, prev.channels[0].length]
      let peak = 0
      for (const ch of prev.channels) {
        for (let i = s; i < e; i++) peak = Math.max(peak, Math.abs(ch[i]))
      }
      if (peak < 1e-9) return prev
      const gain = 1 / peak
      return {
        ...prev,
        channels: prev.channels.map(ch => scaleRange(ch, s, e, gain)),
      }
    }, 'Normalized to peak')
  }

  const normalizeRms = () => {
    if (!sample) return
    commit(prev => {
      const [s, e] = selIndices ?? [0, prev.channels[0].length]
      let sumSq = 0
      let count = 0
      for (const ch of prev.channels) {
        for (let i = s; i < e; i++) { sumSq += ch[i] * ch[i]; count++ }
      }
      if (count === 0) return prev
      const rms = Math.sqrt(sumSq / count)
      if (rms < 1e-9) return prev
      const targetRms = 0.2  // ≈ -14 dB
      const gain = targetRms / rms
      return {
        ...prev,
        channels: prev.channels.map(ch => scaleRange(ch, s, e, gain)),
      }
    }, 'Normalized to -14 dB RMS')
  }

  const applyFade = (inOut: 'in' | 'out', curve: 'linear' | 'exp') => {
    if (!sample) return
    commit(prev => {
      const [s, e] = selIndices ?? [0, prev.channels[0].length]
      const len = e - s
      if (len <= 0) return prev
      return {
        ...prev,
        channels: prev.channels.map(ch => {
          const out = new Float32Array(ch)
          for (let i = 0; i < len; i++) {
            const t = i / Math.max(1, len - 1)
            const g = inOut === 'in'
              ? (curve === 'linear' ? t : t * t)
              : (curve === 'linear' ? 1 - t : (1 - t) * (1 - t))
            out[s + i] = ch[s + i] * g
          }
          return out
        }),
      }
    }, `Fade ${inOut} (${curve})`)
  }

  const reverseSel = () => {
    if (!sample || !selIndices) return
    const [s, e] = selIndices
    commit(prev => ({
      ...prev,
      channels: prev.channels.map(ch => {
        const out = new Float32Array(ch)
        for (let i = s, j = e - 1; i < j; i++, j--) {
          const tmp = out[i]; out[i] = out[j]; out[j] = tmp
        }
        return out
      }),
    }), 'Reversed selection')
  }

  const reverseAll = () => {
    if (!sample) return
    commit(prev => ({
      ...prev,
      channels: prev.channels.map(ch => {
        const out = new Float32Array(ch.length)
        for (let i = 0; i < ch.length; i++) out[i] = ch[ch.length - 1 - i]
        return out
      }),
    }), 'Reversed entire file')
  }

  const applyGain = (db: number) => {
    if (!sample) return
    const g = Math.pow(10, db / 20)
    commit(prev => {
      const [s, e] = selIndices ?? [0, prev.channels[0].length]
      return {
        ...prev,
        channels: prev.channels.map(ch => scaleRange(ch, s, e, g)),
      }
    }, `Gain ${db > 0 ? '+' : ''}${db} dB`)
  }

  const removeDc = () => {
    if (!sample) return
    commit(prev => {
      const [s, e] = selIndices ?? [0, prev.channels[0].length]
      return {
        ...prev,
        channels: prev.channels.map(ch => {
          let sum = 0
          for (let i = s; i < e; i++) sum += ch[i]
          const off = sum / Math.max(1, e - s)
          const out = new Float32Array(ch)
          for (let i = s; i < e; i++) out[i] -= off
          return out
        }),
      }
    }, 'DC offset removed')
  }

  const silenceSel = () => {
    if (!sample || !selIndices) return
    const [s, e] = selIndices
    commit(prev => ({
      ...prev,
      channels: prev.channels.map(ch => {
        const out = new Float32Array(ch)
        for (let i = s; i < e; i++) out[i] = 0
        return out
      }),
    }), 'Silenced selection')
  }

  const trimSilence = () => {
    if (!sample) return
    commit(prev => {
      const threshold = 0.001
      const len = prev.channels[0].length
      let firstNonZero = 0
      outerStart: for (let i = 0; i < len; i++) {
        for (const ch of prev.channels) {
          if (Math.abs(ch[i]) > threshold) { firstNonZero = i; break outerStart }
        }
        firstNonZero = i + 1
      }
      let lastNonZero = len - 1
      outerEnd: for (let i = len - 1; i >= 0; i--) {
        for (const ch of prev.channels) {
          if (Math.abs(ch[i]) > threshold) { lastNonZero = i; break outerEnd }
        }
        lastNonZero = i - 1
      }
      if (firstNonZero >= lastNonZero) return prev
      return {
        ...prev,
        channels: prev.channels.map(ch => ch.slice(firstNonZero, lastNonZero + 1)),
      }
    }, 'Trimmed silence from start and end')
  }

  // ── Export ─────────────────────────────────────────────────────────────────
  const downloadWav = () => {
    if (!sample) return
    const blob = encodeWav(sample.channels, sample.sampleRate)
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    const base = name.replace(/\.[^.]+$/, '')
    a.download = `${base}.edit.wav`
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)
    setBanner({ kind: 'info', text: 'Saved as WAV — check your Downloads folder' })
  }

  // ── Canvas interaction ─────────────────────────────────────────────────────
  const onCanvasMouseDown = (e: React.MouseEvent) => {
    if (!sample) return
    const rect = e.currentTarget.getBoundingClientRect()
    const t = posToTime(e.clientX - rect.left, rect.width, view)
    setSelection({ start: t, end: t })
    const onMove = (ev: MouseEvent) => {
      const t2 = posToTime(ev.clientX - rect.left, rect.width, view)
      setSelection(sel => ({ start: sel.start, end: clamp01(t2) }))
    }
    const onUp = () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }

  const onCanvasWheel = (e: React.WheelEvent) => {
    if (!sample) return
    const zoom = e.deltaY < 0 ? 0.8 : 1.25
    const rect = e.currentTarget.getBoundingClientRect()
    const focus = posToTime(e.clientX - rect.left, rect.width, view)
    const w = (view.end - view.start) * zoom
    setView({
      start: clamp01(focus - (focus - view.start) * zoom),
      end: clamp01(focus - (focus - view.start) * zoom + w),
    })
  }

  // ── Render ────────────────────────────────────────────────────────────────
  const duration = sample ? sample.channels[0].length / sample.sampleRate : 0
  const selDur = sample && selIndices
    ? (selIndices[1] - selIndices[0]) / sample.sampleRate
    : 0

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 15000,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: 'rgba(0,0,0,0.6)', backdropFilter: hw.blur.sm,
      }}
      onMouseDown={onClose}
    >
      <div
        onMouseDown={e => e.stopPropagation()}
        style={{
          width: 'min(920px, 94vw)', maxHeight: '90vh',
          display: 'flex', flexDirection: 'column',
          background: 'rgba(12,12,18,0.98)', border: `1px solid ${hw.borderLight}`,
          borderRadius: hw.radius.lg, boxShadow: '0 20px 60px rgba(0,0,0,0.7)',
          overflow: 'hidden',
        }}
      >
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '10px 14px', background: 'rgba(255,255,255,0.03)',
          borderBottom: `1px solid ${hw.border}`, gap: 12,
        }}>
          <div style={{ display: 'flex', flexDirection: 'column', minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 700, color: hw.accent, letterSpacing: 0.5 }}>
              SAMPLE EDITOR
            </div>
            <div style={{ fontSize: 10, color: hw.textFaint, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {name}
            </div>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <Btn onClick={playing ? stopPlay : play} disabled={!sample} primary>
              {playing ? '■ Stop' : '▶ Play'}
            </Btn>
            <Btn onClick={undo} disabled={undoStack.length === 0}>Undo</Btn>
            <Btn onClick={downloadWav} disabled={!sample}>Save as WAV</Btn>
            <Btn onClick={onClose}>Close</Btn>
          </div>
        </div>

        {banner && (
          <div style={{
            padding: '6px 14px', fontSize: 11,
            background: banner.kind === 'error' ? 'rgba(239,68,68,0.12)' : hw.accentDim,
            borderBottom: `1px solid ${banner.kind === 'error' ? 'rgba(239,68,68,0.4)' : hw.accentGlow}`,
            color: banner.kind === 'error' ? hw.red : hw.accentLight,
            display: 'flex', alignItems: 'center', gap: 8,
          }}>
            <span style={{ flex: 1 }}>{banner.text}</span>
            <span onClick={() => setBanner(null)} style={{ cursor: 'pointer', opacity: 0.7 }}>✕</span>
          </div>
        )}

        {/* Waveform canvas */}
        <div style={{ flex: 1, position: 'relative', padding: 10, background: hw.bg }}>
          {loading && (
            <div style={{ color: hw.textMuted, fontSize: 11, padding: 24 }}>Loading audio…</div>
          )}
          {err && (
            <div style={{ color: hw.red, fontSize: 11, padding: 24 }}>
              Could not decode: {err}
            </div>
          )}
          {!loading && !err && sample && (
            <canvas
              ref={canvasRef}
              width={880}
              height={220}
              style={{
                width: '100%', height: 220,
                background: hw.bgCanvasDark,
                border: `1px solid ${hw.borderDark}`,
                borderRadius: hw.radius.sm,
                cursor: 'crosshair',
                display: 'block',
              }}
              onMouseDown={onCanvasMouseDown}
              onWheel={onCanvasWheel}
            />
          )}

          {/* Zoom controls */}
          {sample && (
            <div style={{
              display: 'flex', alignItems: 'center', gap: 10,
              marginTop: 8, fontSize: 10, color: hw.textMuted,
            }}>
              <span>Zoom: scroll on waveform · view {formatRange(
                Math.round((view.end - view.start) * sample.channels[0].length),
                sample.sampleRate,
              )}</span>
              <div style={{ flex: 1 }} />
              <Btn onClick={() => setView({ start: 0, end: 1 })}>Fit</Btn>
              <Btn onClick={() => {
                if (!selIndices || !sample) return
                const n = sample.channels[0].length
                setView({ start: selIndices[0] / n, end: selIndices[1] / n })
              }} disabled={!hasSelection}>Zoom to selection</Btn>
              <span style={{ width: 12 }} />
              <span>{hasSelection
                ? `Selection ${formatRange(selIndices![1] - selIndices![0], sample.sampleRate)}`
                : 'No selection'}</span>
              <Btn onClick={selectAll}>Select all</Btn>
              <Btn onClick={deselect} disabled={!hasSelection}>Deselect</Btn>
            </div>
          )}
        </div>

        {/* Toolbar */}
        <div style={{
          padding: '10px 14px',
          background: 'rgba(255,255,255,0.02)',
          borderTop: `1px solid ${hw.border}`,
          display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 6,
        }}>
          <ToolbarGroup label="Clipboard">
            <Btn onClick={cutSel} disabled={!hasSelection}>Cut</Btn>
            <Btn onClick={copySel} disabled={!hasSelection}>Copy</Btn>
            <Btn onClick={pasteAtCursor} disabled={!clipboard}>Paste</Btn>
            <Btn onClick={deleteSel} disabled={!hasSelection}>Delete</Btn>
          </ToolbarGroup>
          <ToolbarGroup label="Normalize & gain">
            <Btn onClick={normalizePeak}>Peak</Btn>
            <Btn onClick={normalizeRms}>RMS</Btn>
            <Btn onClick={() => applyGain(+3)}>+3 dB</Btn>
            <Btn onClick={() => applyGain(-3)}>-3 dB</Btn>
          </ToolbarGroup>
          <ToolbarGroup label="Fades">
            <Btn onClick={() => applyFade('in',  'linear')} disabled={!hasSelection}>In lin</Btn>
            <Btn onClick={() => applyFade('in',  'exp')}    disabled={!hasSelection}>In exp</Btn>
            <Btn onClick={() => applyFade('out', 'linear')} disabled={!hasSelection}>Out lin</Btn>
            <Btn onClick={() => applyFade('out', 'exp')}    disabled={!hasSelection}>Out exp</Btn>
          </ToolbarGroup>
          <ToolbarGroup label="Shape">
            <Btn onClick={reverseSel} disabled={!hasSelection}>Rev sel</Btn>
            <Btn onClick={reverseAll}>Rev all</Btn>
            <Btn onClick={silenceSel} disabled={!hasSelection}>Silence</Btn>
            <Btn onClick={removeDc}>DC off</Btn>
            <Btn onClick={trimSilence}>Trim sil</Btn>
          </ToolbarGroup>
        </div>

        {/* Footer */}
        <div style={{
          display: 'flex', alignItems: 'center',
          padding: '6px 14px', gap: 16,
          background: hw.bgPanel,
          borderTop: `1px solid ${hw.border}`,
          fontSize: 10, color: hw.textMuted,
        }}>
          {sample ? (
            <>
              <span>{sample.channels.length === 1 ? 'Mono' : 'Stereo'}</span>
              <span>{sample.sampleRate} Hz</span>
              <span>{duration.toFixed(3)} s</span>
              {hasSelection && <span style={{ color: hw.accent }}>sel {selDur.toFixed(3)} s</span>}
              <span style={{ flex: 1 }} />
              <span>Scroll to zoom · drag to select · Fades and reverse act on selection</span>
            </>
          ) : (
            <span>Waiting for audio data…</span>
          )}
        </div>
      </div>
    </div>
  )
}

// ── Helpers ─────────────────────────────────────────────────────────────────

function Btn({
  onClick, disabled, primary, children,
}: { onClick: () => void; disabled?: boolean; primary?: boolean; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      style={{
        padding: '4px 10px', fontSize: 10, fontWeight: 600,
        color: disabled ? hw.textFaint : primary ? '#fff' : hw.textPrimary,
        background: disabled
          ? hw.bgElevated
          : primary ? hw.accent : 'rgba(255,255,255,0.04)',
        border: `1px solid ${primary && !disabled ? hw.accent : hw.border}`,
        borderRadius: hw.radius.sm,
        cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.4 : 1,
      }}
    >
      {children}
    </button>
  )
}

function ToolbarGroup({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <div style={{
        fontSize: 8, color: hw.textFaint, letterSpacing: 0.6,
        textTransform: 'uppercase', marginBottom: 4, padding: '0 2px',
      }}>{label}</div>
      <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>{children}</div>
    </div>
  )
}

function clamp01(x: number) { return Math.max(0, Math.min(1, x)) }
function posToTime(px: number, width: number, view: Selection) {
  const t = view.start + (px / Math.max(1, width)) * (view.end - view.start)
  return clamp01(t)
}

function cloneSample(s: Sample): Sample {
  return { sampleRate: s.sampleRate, channels: s.channels.map(c => new Float32Array(c)) }
}

function sliceRemove(arr: Float32Array, s: number, e: number): Float32Array {
  const out = new Float32Array(arr.length - (e - s))
  out.set(arr.subarray(0, s), 0)
  out.set(arr.subarray(e), s)
  return out
}

function scaleRange(arr: Float32Array, s: number, e: number, g: number): Float32Array {
  const out = new Float32Array(arr)
  for (let i = s; i < e; i++) out[i] = Math.max(-1, Math.min(1, out[i] * g))
  return out
}

function formatRange(samples: number, sampleRate: number): string {
  const sec = samples / sampleRate
  if (sec < 0.01) return `${samples} samp`
  if (sec < 1) return `${(sec * 1000).toFixed(0)} ms`
  return `${sec.toFixed(2)} s`
}

function drawWaveform(canvas: HTMLCanvasElement, sample: Sample, view: Selection, sel: Selection) {
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  const w = canvas.width
  const h = canvas.height
  ctx.clearRect(0, 0, w, h)

  // Background
  ctx.fillStyle = hw.bgCanvasDark
  ctx.fillRect(0, 0, w, h)

  // Center axis
  const n = sample.channels[0].length
  const perChannelH = h / sample.channels.length
  const startIdx = Math.floor(view.start * n)
  const endIdx = Math.floor(view.end * n)
  const viewSpan = Math.max(1, endIdx - startIdx)
  const samplesPerPx = viewSpan / w

  ctx.fillStyle = hw.accent
  for (let c = 0; c < sample.channels.length; c++) {
    const ch = sample.channels[c]
    const yMid = perChannelH * (c + 0.5)
    const ampScale = perChannelH * 0.45
    // Zero line
    ctx.fillStyle = 'rgba(255,255,255,0.08)'
    ctx.fillRect(0, yMid, w, 1)
    // Samples
    ctx.fillStyle = hw.accentLight
    for (let x = 0; x < w; x++) {
      const s = startIdx + Math.floor(x * samplesPerPx)
      const e = Math.min(ch.length, startIdx + Math.floor((x + 1) * samplesPerPx))
      let min = 0, max = 0
      for (let i = s; i < e; i++) {
        const v = ch[i]
        if (v < min) min = v
        if (v > max) max = v
      }
      const y1 = yMid - max * ampScale
      const y2 = yMid - min * ampScale
      ctx.fillRect(x, y1, 1, Math.max(1, y2 - y1))
    }
  }

  // Selection overlay
  if (Math.abs(sel.end - sel.start) > 1e-6) {
    const s = normSel(sel)
    const viewLen = view.end - view.start
    if (viewLen > 0) {
      const x1 = Math.max(0, ((s.start - view.start) / viewLen) * w)
      const x2 = Math.min(w, ((s.end - view.start) / viewLen) * w)
      if (x2 > x1) {
        ctx.fillStyle = 'rgba(220,38,38,0.18)'
        ctx.fillRect(x1, 0, x2 - x1, h)
        ctx.fillStyle = hw.accent
        ctx.fillRect(x1, 0, 1, h)
        ctx.fillRect(x2 - 1, 0, 1, h)
      }
    }
  }
}
