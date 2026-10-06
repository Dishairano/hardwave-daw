import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'
import { useNotificationStore } from '../../stores/notificationStore'

/**
 * Painting something out of a recording.
 *
 * A cough in a vocal take or a chair creak under a verse is mixed in
 * with everything else in time, but on a spectrogram it sits in its
 * own patch of time and frequency. Drag a box around it and it is
 * rubbed out, at whatever strength is set, because a hole can be as
 * noticeable as the noise was.
 */

interface SpectrogramData {
  frames: number[][]
  bins: number
  hop: number
  window: number
  sampleRate: number
  lengthSamples: number
}

interface Box {
  x0: number; y0: number; x1: number; y1: number
}

const FLOOR_DB = -90
const CEILING_DB = -6

export function SpectralEditor({
  trackId, clipId, onClose,
}: { trackId: string; clipId: string; onClose: () => void }) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const [data, setData] = useState<SpectrogramData | null>(null)
  const [boxes, setBoxes] = useState<Box[]>([])
  const [dragging, setDragging] = useState<Box | null>(null)
  const [strength, setStrength] = useState(1)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    invoke<SpectrogramData>('clip_spectrogram', { trackId, clipId })
      .then(setData)
      .catch(e => setError(String(e)))
  }, [trackId, clipId])

  // Draw the spectrogram, then whatever has been painted on it.
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas || !data) return
    const width = canvas.width
    const height = canvas.height
    const context = canvas.getContext('2d')
    if (!context) return

    const image = context.createImageData(width, height)
    const frames = data.frames.length || 1
    for (let x = 0; x < width; x++) {
      const frame = data.frames[Math.min(frames - 1, Math.floor(x / width * frames))] ?? []
      for (let y = 0; y < height; y++) {
        // Low frequencies at the bottom, and on a log scale, because
        // that is where the ear and the music both live.
        const normalised = 1 - y / height
        const bin = Math.min(data.bins - 1, Math.round((Math.pow(1000, normalised) - 1) / 999 * (data.bins - 1)))
        const db = frame[bin] ?? FLOOR_DB
        const level = Math.max(0, Math.min(1, (db - FLOOR_DB) / (CEILING_DB - FLOOR_DB)))
        const index = (y * width + x) * 4
        image.data[index] = Math.round(255 * Math.pow(level, 0.8))
        image.data[index + 1] = Math.round(90 * level)
        image.data[index + 2] = Math.round(120 * Math.pow(level, 2))
        image.data[index + 3] = 255
      }
    }
    context.putImageData(image, 0, 0)

    const paint = (box: Box, live: boolean) => {
      context.strokeStyle = live ? '#fff' : hw.accent
      context.fillStyle = live ? 'rgba(255,255,255,0.15)' : 'rgba(239,68,68,0.22)'
      const x = Math.min(box.x0, box.x1)
      const y = Math.min(box.y0, box.y1)
      const w = Math.abs(box.x1 - box.x0)
      const h = Math.abs(box.y1 - box.y0)
      context.fillRect(x, y, w, h)
      context.strokeRect(x, y, w, h)
    }
    for (const box of boxes) paint(box, false)
    if (dragging) paint(dragging, true)
  }, [data, boxes, dragging])

  const pointIn = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect()
    return {
      x: (e.clientX - rect.left) / rect.width * e.currentTarget.width,
      y: (e.clientY - rect.top) / rect.height * e.currentTarget.height,
    }
  }

  const apply = useCallback(async () => {
    if (!data || boxes.length === 0) return
    const canvas = canvasRef.current
    if (!canvas) return
    setBusy(true)
    const nyquist = data.sampleRate / 2
    const patches = boxes.map(box => {
      const x0 = Math.min(box.x0, box.x1) / canvas.width
      const x1 = Math.max(box.x0, box.x1) / canvas.width
      // The drawing is log in frequency and the engine wants hertz.
      const toHz = (y: number) => {
        const normalised = 1 - y / canvas.height
        const bin = (Math.pow(1000, normalised) - 1) / 999
        return bin * nyquist
      }
      const low = toHz(Math.max(box.y0, box.y1))
      const high = toHz(Math.min(box.y0, box.y1))
      return {
        startSample: Math.round(x0 * data.lengthSamples),
        endSample: Math.round(x1 * data.lengthSamples),
        lowHz: low,
        highHz: high,
        strength,
      }
    })
    try {
      await invoke('erase_from_clip', { trackId, clipId, patches })
      await useTrackStore.getState().fetchTracks()
      useNotificationStore.getState().push('info',
        `${patches.length} ${patches.length === 1 ? 'patch' : 'patches'} painted out`, {
          detail: 'The take as it was recorded is kept: this is a new file.',
        })
      onClose()
    } catch (e) {
      setError(String(e))
      setBusy(false)
    }
  }, [data, boxes, strength, trackId, clipId, onClose])

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 9800,
        background: 'rgba(0,0,0,0.5)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div style={{
        width: 900, maxWidth: '96vw',
        background: hw.bg, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg, overflow: 'hidden',
      }}>
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 12,
          background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Paint it out</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>
            drag a box around the cough, the creak or the click
          </div>
          <div style={{ flex: 1 }} />
          <button onClick={onClose} style={btn()}>Close</button>
        </div>

        <div style={{ padding: 12, display: 'flex', flexDirection: 'column', gap: 10 }}>
          <canvas
            ref={canvasRef}
            width={860}
            height={320}
            onMouseDown={e => { const p = pointIn(e); setDragging({ x0: p.x, y0: p.y, x1: p.x, y1: p.y }) }}
            onMouseMove={e => {
              if (!dragging) return
              const p = pointIn(e)
              setDragging({ ...dragging, x1: p.x, y1: p.y })
            }}
            onMouseUp={() => {
              if (!dragging) return
              const big = Math.abs(dragging.x1 - dragging.x0) > 3 && Math.abs(dragging.y1 - dragging.y0) > 3
              if (big) setBoxes(b => [...b, dragging])
              setDragging(null)
            }}
            onMouseLeave={() => setDragging(null)}
            style={{
              width: '100%', height: 320, cursor: 'crosshair',
              background: '#07070b', borderRadius: hw.radius.sm,
              border: `1px solid ${hw.border}`,
            }}
          />

          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <span style={{ fontSize: 11, color: hw.textSecondary }}>Strength</span>
            <input
              type="range" min={0.1} max={1} step={0.05} value={strength}
              onChange={e => setStrength(Number(e.target.value))}
              style={{ width: 180 }}
            />
            <span style={{ fontSize: 10, color: hw.textFaint }}>
              {Math.round(strength * 100)}% — part way is often better than a hole
            </span>
            <div style={{ flex: 1 }} />
            <span style={{ fontSize: 10, color: error ? hw.red : hw.textFaint }}>
              {error ?? `${boxes.length} painted`}
            </span>
            <button onClick={() => setBoxes([])} style={btn()}>Clear</button>
            <button
              onClick={() => void apply()}
              disabled={busy || boxes.length === 0}
              style={{ ...btn(), background: hw.accent, color: '#fff' }}
            >{busy ? 'Working…' : 'Paint it out'}</button>
          </div>
        </div>
      </div>
    </div>
  )
}

function btn(): React.CSSProperties {
  return {
    padding: '4px 10px', fontSize: 11, fontWeight: 600,
    background: 'rgba(255,255,255,0.08)', color: hw.textSecondary,
    border: 'none', borderRadius: hw.radius.sm, cursor: 'pointer', fontFamily: 'inherit',
  }
}
