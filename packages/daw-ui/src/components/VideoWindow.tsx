import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useTransportStore } from '../stores/transportStore'
import { useNotificationStore } from '../stores/notificationStore'

/**
 * Scoring to picture.
 *
 * A film or a game capture, played against the song, so a hit can be
 * written where the cut is rather than where it sounded about right.
 * The picture is the window's job and the clock is the engine's: the
 * playhead drives the video, never the other way round, because a
 * frame arriving late must not drag the song with it.
 */

interface VideoStatus {
  path: string
  offsetTicks: number
  muted: boolean
}

const PPQ = 960
/** Closer than this and a seek would be worse than the drift. */
const SLIP_SECONDS = 0.08

export function VideoWindow({ onClose }: { onClose: () => void }) {
  const videoRef = useRef<HTMLVideoElement | null>(null)
  const [status, setStatus] = useState<VideoStatus | null>(null)
  const [src, setSrc] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const playing = useTransportStore(s => s.playing)
  const positionSamples = useTransportStore(s => s.positionSamples)
  const sampleRate = useTransportStore(s => s.sampleRate)
  const bpm = useTransportStore(s => s.bpm)

  const load = useCallback(async (next: VideoStatus | null) => {
    setStatus(next)
    if (!next) { setSrc(null); return }
    try {
      const { convertFileSrc } = await import('@tauri-apps/api/core')
      setSrc(convertFileSrc(next.path))
    } catch {
      setSrc(null)
    }
  }, [])

  useEffect(() => {
    invoke<VideoStatus | null>('get_video')
      .then(v => void load(v))
      .catch(() => { /* no engine in the browser preview */ })
  }, [load])

  // Where the film should be, in its own seconds.
  const wantedSeconds = (() => {
    const offsetSeconds = status
      ? (status.offsetTicks / PPQ) * 60 / Math.max(1, bpm)
      : 0
    return positionSamples / (sampleRate || 48000) - offsetSeconds
  })()

  useEffect(() => {
    const video = videoRef.current
    if (!video || !src) return
    if (wantedSeconds < 0) {
      // Before the film starts: hold the first frame.
      if (!video.paused) video.pause()
      if (video.currentTime !== 0) video.currentTime = 0
      return
    }
    // Only seek when it has actually drifted, or every poll would
    // stutter the picture.
    if (Math.abs(video.currentTime - wantedSeconds) > SLIP_SECONDS) {
      video.currentTime = wantedSeconds
    }
    if (playing && video.paused) void video.play().catch(() => {})
    if (!playing && !video.paused) video.pause()
  }, [wantedSeconds, playing, src])

  const pick = useCallback(async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const picked = await open({
        multiple: false,
        filters: [{ name: 'Video', extensions: ['mp4', 'webm', 'mov', 'm4v', 'mkv'] }],
      })
      if (typeof picked !== 'string') return
      const next = await invoke<VideoStatus>('set_video', { path: picked })
      await load(next)
      setError(null)
    } catch (e) {
      setError(String(e))
      useNotificationStore.getState().push('warning', 'Could not open that video', { detail: String(e) })
    }
  }, [load])

  return (
    <div
      style={{
        position: 'fixed', right: 24, bottom: 24, zIndex: 9600,
        width: 420, background: hw.bg, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg,
        overflow: 'hidden', boxShadow: '0 18px 48px rgba(0,0,0,0.5)',
      }}
    >
      <div style={{
        padding: '6px 10px', display: 'flex', alignItems: 'center', gap: 10,
        background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
      }}>
        <div style={{ fontSize: 11, fontWeight: 600 }}>Video</div>
        <div style={{ fontSize: 9, color: hw.textFaint }}>follows the playhead</div>
        <div style={{ flex: 1 }} />
        <button onClick={() => void pick()} style={btn()}>{status ? 'Replace' : 'Open…'}</button>
        {status && (
          <button
            onClick={() => {
              const next = !status.muted
              setStatus({ ...status, muted: next })
              invoke('set_video_muted', { muted: next }).catch(() => {})
            }}
            style={btn()}
          >{status.muted ? 'Sound off' : 'Sound on'}</button>
        )}
        {status && (
          <button
            onClick={() => { invoke('clear_video').catch(() => {}); void load(null) }}
            style={btn()}
          >Remove</button>
        )}
        <button onClick={onClose} style={btn()}>Close</button>
      </div>

      {src ? (
        <video
          ref={videoRef}
          src={src}
          muted={status?.muted ?? true}
          playsInline
          style={{ width: '100%', display: 'block', background: '#000' }}
        />
      ) : (
        <div style={{ padding: 28, textAlign: 'center', fontSize: 11, color: hw.textFaint }}>
          {error ?? 'Open a film or a capture and it plays against the song.'}
        </div>
      )}

      {status && (
        <div style={{
          padding: '6px 10px', display: 'flex', alignItems: 'center', gap: 8,
          borderTop: `1px solid ${hw.border}`, fontSize: 10, color: hw.textFaint,
        }}>
          <span>Starts at bar</span>
          <input
            type="number"
            min={1}
            value={Math.floor(status.offsetTicks / (PPQ * 4)) + 1}
            onChange={e => {
              const bar = Math.max(1, Number(e.target.value) || 1)
              const offsetTicks = (bar - 1) * PPQ * 4
              setStatus({ ...status, offsetTicks })
              invoke('set_video_offset', { offsetTicks }).catch(() => {})
            }}
            style={{
              width: 64, padding: '2px 6px', fontSize: 10, fontFamily: 'inherit',
              background: 'rgba(255,255,255,0.04)', color: hw.textPrimary,
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
            }}
          />
          <div style={{ flex: 1 }} />
          <span>{wantedSeconds < 0 ? 'before the film' : `${wantedSeconds.toFixed(2)} s in`}</span>
        </div>
      )}
    </div>
  )
}

function btn(): React.CSSProperties {
  return {
    padding: '3px 8px', fontSize: 10, fontWeight: 600,
    background: 'rgba(255,255,255,0.08)', color: hw.textSecondary,
    border: 'none', borderRadius: hw.radius.sm, cursor: 'pointer', fontFamily: 'inherit',
  }
}
