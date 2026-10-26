import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { hw } from '../../theme'
import { useProjectStore } from '../../stores/projectStore'
import { useTrackStore } from '../../stores/trackStore'
import { useNotificationStore } from '../../stores/notificationStore'
import { DialogFrame } from '../ui/DialogFrame'
import { btn } from '../ui/dialogStyles'
import { t, tf, useLanguage } from '../../i18n'

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
  useLanguage()
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
      useNotificationStore.getState().push('info', tf('{name} separated', { name: clipName }), {
        detail: minutes === 1
          ? tf('Drums, bass, vocals and the rest are on four tracks under it, and the clip is muted. Took about 1 minute. The files are in {folder}.',
            { folder: result.folder })
          : tf('Drums, bass, vocals and the rest are on four tracks under it, and the clip is muted. Took about {n} minutes. The files are in {folder}.',
            { n: minutes, folder: result.folder }),
      })
      onClose()
    } catch (e) {
      const reason = String(e)
      if (reason === 'stopped') { onClose(); return }
      if (!open.current) {
        useNotificationStore.getState().push('warning', tf('Could not separate {name}', { name: clipName }), { detail: reason })
        return
      }
      setError(reason)
      setPhase('failed')
    }
  }

  const stop = () => { void invoke('stop_stems') }

  const line = (() => {
    if (!progress) return t('Getting ready…')
    switch (progress.stage) {
      case 'sending': return t('Sending the clip…')
      case 'waiting': return progress.ahead <= 1
        ? t('Next in line. One song is being separated before yours.')
        : tf('{n} songs ahead of yours.', { n: progress.ahead })
      case 'separating': return tf('Separating: {n}%', { n: Math.round(progress.progress * 100) })
      case 'receiving': return t('Bringing the parts down…')
    }
  })()

  return (
    <DialogFrame
      title={t('Separate stems')}
      subtitle={clipName}
      onClose={onClose}
      width={460}
      headerActions={<>
          <span style={{
            fontSize: 11, fontWeight: 700, letterSpacing: 0.6, padding: '2px 6px',
            borderRadius: 4, background: hw.accent, color: '#fff',
          }}>PRO</span>
      </>}
    >
      <div style={{ padding: 16, fontSize: 12.5, lineHeight: 1.6 }}>
        {phase === 'ready' && (
          <>
            <p style={{ margin: '0 0 10px', color: hw.textSecondary }}>
              {t('Splits this clip into drums, bass, vocals and the rest. Each part goes on its own track under this one, in the same place, and the clip is muted, so playback sounds the same until you change something.')}
            </p>
            <p style={{ margin: '0 0 14px', color: hw.textFaint }}>
              {t("The clip's audio file is sent to our server and separated there, one song at a time. Expect a few minutes for a whole song; you can keep working meanwhile. The file is deleted from the server within a day.")}
            </p>
            <div style={{ display: 'flex', gap: 8 }}>
              <button onClick={() => void start()} style={btn(true)}>{t('Separate')}</button>
              <button onClick={onClose} style={btn(false)}>{t('Cancel')}</button>
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
              <button onClick={onClose} style={btn(false)} title={t('It keeps going; the parts land when they are ready')}>
                {t('Keep working')}
              </button>
              <button onClick={stop} style={btn(false)}>{t('Stop')}</button>
            </div>
          </>
        )}

        {phase === 'failed' && (
          <>
            <div style={{ color: error === PRO_LINE ? hw.textPrimary : hw.red, marginBottom: 10 }}>{error === PRO_LINE ? t(PRO_LINE) : error}</div>
            {error === PRO_LINE && (
              <p style={{ margin: '0 0 12px', color: hw.textSecondary }}>
                {t('The DAW and the Hardwave plug-ins in it are free. Pro adds what runs on our servers, like this, and the plug-ins in other DAWs.')}
              </p>
            )}
            <div style={{ display: 'flex', gap: 8 }}>
              {error === PRO_LINE
                ? <button onClick={() => window.open('https://hardwavestudios.com/pricing', '_blank', 'noopener,noreferrer')} style={btn(true)}>{t('See Pro')}</button>
                : <button onClick={() => void start()} style={btn(true)}>{t('Try again')}</button>}
              <button onClick={onClose} style={btn(false)}>{t('Close')}</button>
            </div>
          </>
        )}
      </div>
    </DialogFrame>
  )
}

