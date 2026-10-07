import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { hw } from '../../theme'
import { useProjectStore } from '../../stores/projectStore'
import { useTrackStore } from '../../stores/trackStore'
import { useNotificationStore } from '../../stores/notificationStore'

/**
 * Separate a clip into drums, bass, vocals and the rest.
 *
 * The clip's audio goes to our server, which separates one song at a
 * time; this shows where it is in the line and how far along. When it
 * is done the four parts land on tracks under the clip and the clip is
 * muted. Pro, because it runs on our machine rather than this one.
 */

interface Progress {
  stage: 'sending' | 'waiting' | 'separating' | 'receiving'
  ahead: number
  progress: number
}

interface Props {
  trackId: string
  clipId: string
  clipName: string
  onClose: () => void
}

const PRO_LINE = 'Stem separation is part of Hardwave Pro.'

export function StemsDialog({ trackId, clipId, clipName, onClose }: Props) {
  const [phase, setPhase] = useState<'ready' | 'working' | 'failed'>('ready')
  const [progress, setProgress] = useState<Progress | null>(null)
  const [error, setError] = useState<string | null>(null)
  const started = useRef(0)
  // "Keep working" closes the window while the separation carries on;
  // a failure after that is told as a notification instead.
  const open = useRef(true)

  useEffect(() => {
    open.current = true
    const unlisten = listen<Progress>('stems-progress', e => setProgress(e.payload))
    return () => {
      open.current = false
      void unlisten.then(f => f())
    }
  }, [])

  const start = async () => {
    setPhase('working')
    setError(null)
    setProgress(null)
    started.current = Date.now()
    try {
      const result = await invoke<{ tracks: string[]; folder: string }>('separate_stems', {
        trackId, clipId, projectPath: useProjectStore.getState().filePath,
      })
      await useTrackStore.getState().fetchTracks()
      useProjectStore.setState({ dirty: true })
      const minutes = Math.max(1, Math.round((Date.now() - started.current) / 60000))
      useNotificationStore.getState().push('info', `${clipName} separated`, {
        detail: `Drums, bass, vocals and the rest are on four tracks under it, and the clip is muted. `
          + `Took about ${minutes} ${minutes === 1 ? 'minute' : 'minutes'}. The files are in ${result.folder}.`,
      })
      onClose()
    } catch (e) {
      const reason = String(e)
      if (reason === 'stopped') { onClose(); return }
      if (!open.current) {
        useNotificationStore.getState().push('warning', `Could not separate ${clipName}`, { detail: reason })
        return
      }
      setError(reason)
      setPhase('failed')
    }
  }

  const stop = () => { void invoke('stop_stems') }

  const line = (() => {
    if (!progress) return 'Getting ready…'
    switch (progress.stage) {
      case 'sending': return 'Sending the clip…'
      case 'waiting': return progress.ahead <= 1
        ? 'Next in line. One song is being separated before yours.'
        : `${progress.ahead} songs ahead of yours.`
      case 'separating': return `Separating: ${Math.round(progress.progress * 100)}%`
      case 'receiving': return 'Bringing the parts down…'
    }
  })()

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 9800, background: 'rgba(0,0,0,0.45)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget && phase !== 'working') onClose() }}
    >
      <div style={{
        width: 460, maxWidth: '94vw', background: hw.bg, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg, overflow: 'hidden',
      }}>
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 12,
          background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Separate stems</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>{clipName}</div>
          <div style={{ flex: 1 }} />
          <span style={{
            fontSize: 9, fontWeight: 700, letterSpacing: 0.6, padding: '2px 6px',
            borderRadius: 4, background: hw.accent, color: '#fff',
          }}>PRO</span>
        </div>

        <div style={{ padding: 16, fontSize: 11, lineHeight: 1.6 }}>
          {phase === 'ready' && (
            <>
              <p style={{ margin: '0 0 10px', color: hw.textSecondary }}>
                Splits this clip into drums, bass, vocals and the rest. Each part goes on its own
                track under this one, in the same place, and the clip is muted, so playback sounds
                the same until you change something.
              </p>
              <p style={{ margin: '0 0 14px', color: hw.textFaint }}>
                The clip's audio file is sent to our server and separated there, one song at a
                time. Expect a few minutes for a whole song; you can keep working meanwhile. The
                file is deleted from the server within a day.
              </p>
              <div style={{ display: 'flex', gap: 8 }}>
                <button onClick={() => void start()} style={btn(true)}>Separate</button>
                <button onClick={onClose} style={btn(false)}>Cancel</button>
              </div>
            </>
          )}

          {phase === 'working' && (
            <>
              <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 8 }}>{line}</div>
              <div style={{ height: 6, borderRadius: 3, background: 'rgba(255,255,255,0.06)', overflow: 'hidden', marginBottom: 12 }}>
                <div style={{
                  height: '100%', background: hw.accent, transition: 'width 0.4s',
                  width: `${progress?.stage === 'separating' ? Math.round(progress.progress * 100)
                    : progress?.stage === 'receiving' ? 100 : progress?.stage === 'waiting' ? 4 : 2}%`,
                }} />
              </div>
              <div style={{ display: 'flex', gap: 8 }}>
                <button onClick={onClose} style={btn(false)} title="It keeps going; the parts land when they are ready">
                  Keep working
                </button>
                <button onClick={stop} style={btn(false)}>Stop</button>
              </div>
            </>
          )}

          {phase === 'failed' && (
            <>
              <div style={{ color: error === PRO_LINE ? hw.textPrimary : hw.red, marginBottom: 10 }}>{error}</div>
              {error === PRO_LINE && (
                <p style={{ margin: '0 0 12px', color: hw.textSecondary }}>
                  The DAW and the Hardwave plug-ins in it are free. Pro adds what runs on our
                  servers, like this, and the plug-ins in other DAWs.
                </p>
              )}
              <div style={{ display: 'flex', gap: 8 }}>
                {error === PRO_LINE
                  ? <button onClick={() => window.open('https://hardwavestudios.com/pricing', '_blank', 'noopener,noreferrer')} style={btn(true)}>See Pro</button>
                  : <button onClick={() => void start()} style={btn(true)}>Try again</button>}
                <button onClick={onClose} style={btn(false)}>Close</button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  )
}

function btn(primary: boolean): React.CSSProperties {
  return {
    padding: '6px 12px', fontSize: 11, fontWeight: 600,
    background: primary ? hw.accent : 'rgba(255,255,255,0.08)',
    color: primary ? '#fff' : hw.textSecondary,
    border: 'none', borderRadius: hw.radius.sm, cursor: 'pointer', fontFamily: 'inherit',
  }
}
