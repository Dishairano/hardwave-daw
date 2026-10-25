import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'

interface MtcStatus {
  enabled: boolean
  offsetSeconds: number
  seenSeconds: number | null
  fps: number | null
}

/** `01:00:12:05` from seconds, at the master's frame rate. */
export function formatTimecode(seconds: number, fps: number): string {
  const whole = Math.floor(seconds)
  // A hair over, so 5/25 of a second reads as frame 5, not 4.999.
  const frames = Math.floor((seconds - whole) * fps + 1e-6)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${pad(Math.floor(whole / 3600))}:${pad(Math.floor(whole / 60) % 60)}:${pad(whole % 60)}:${pad(frames)}`
}

/** `01:00:00` to seconds, or null when it does not read as a time. */
export function parseStart(text: string): number | null {
  const m = /^\s*(\d{1,2}):(\d{2}):(\d{2})\s*$/.exec(text)
  if (!m) return null
  const [h, min, s] = [Number(m[1]), Number(m[2]), Number(m[3])]
  if (min > 59 || s > 59) return null
  return h * 3600 + min * 60 + s
}

/**
 * Follow MIDI timecode from a master: a video app, a tape machine or
 * another DAW. The song plays while the master's timecode comes in, moves
 * when it jumps, and stops when it stops. The start time is the master's
 * timecode at the top of the song; video work usually starts at 01:00:00.
 */
export function MtcFollowRow({ disabled }: { disabled: boolean }) {
  const [status, setStatus] = useState<MtcStatus | null>(null)
  const [start, setStart] = useState('00:00:00')

  useEffect(() => {
    let alive = true
    const read = () => {
      invoke<MtcStatus>('get_mtc_status')
        .then(s => { if (alive) setStatus(s) })
        .catch(() => { /* no MIDI */ })
    }
    read()
    invoke<MtcStatus>('get_mtc_status').then(s => {
      if (alive) setStart(formatTimecode(s.offsetSeconds, 30).slice(0, 8))
    }).catch(() => {})
    const id = window.setInterval(read, 500)
    return () => { alive = false; window.clearInterval(id) }
  }, [])

  const apply = (enabled: boolean, text = start) => {
    const offsetSeconds = parseStart(text)
    if (offsetSeconds === null) return
    invoke('set_mtc_follow', { enabled, offsetSeconds })
      .then(() => setStatus(s => (s ? { ...s, enabled, offsetSeconds } : s)))
      .catch(() => {})
  }

  const enabled = status?.enabled ?? false
  const seen = status?.seenSeconds != null && status.fps
    ? `Master: ${formatTimecode(status.seenSeconds, status.fps)} at ${Number(status.fps.toFixed(2))} fps`
    : 'No timecode received yet'

  return (
    <div style={{
      marginTop: 6,
      display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8,
      padding: '6px 8px',
      background: 'rgba(0,0,0,0.25)',
      borderRadius: hw.radius.sm,
      border: `1px solid ${enabled ? hw.accent : hw.borderDark}`,
      opacity: disabled ? 0.6 : 1,
    }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
        <span style={{ fontSize: 11, color: hw.textSecondary }}>Follow MIDI timecode (MTC)</span>
        <span style={{ fontSize: 9, color: hw.textFaint, fontFamily: "'Consolas', monospace" }}>{seen}</span>
      </div>
      <label style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 9, color: hw.textMuted, flexShrink: 0 }}
        title="The master's timecode at the top of the song. Video work usually starts at 01:00:00.">
        Song starts at
        <input
          value={start}
          onChange={e => setStart(e.target.value)}
          onBlur={() => { if (parseStart(start) !== null) apply(enabled) }}
          style={{
            width: 58, fontSize: 10, padding: '1px 4px', fontFamily: "'Consolas', monospace",
            background: 'rgba(255,255,255,0.04)', color: hw.textPrimary,
            border: `1px solid ${parseStart(start) === null ? hw.red : hw.borderDark}`, borderRadius: hw.radius.sm,
          }}
        />
      </label>
      <button
        onClick={() => apply(!enabled)}
        disabled={disabled}
        style={{
          padding: '2px 10px', fontSize: 10, fontWeight: 600,
          borderRadius: hw.radius.sm, border: 'none', flexShrink: 0,
          cursor: disabled ? 'default' : 'pointer',
          background: enabled ? hw.accent : 'rgba(255,255,255,0.08)',
          color: enabled ? '#fff' : hw.textSecondary,
          fontFamily: 'inherit',
        }}
      >
        {enabled ? 'On' : 'Off'}
      </button>
    </div>
  )
}
