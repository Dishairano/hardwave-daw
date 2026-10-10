import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useMeterStore } from '../stores/meterStore'
import { useNotificationStore } from '../stores/notificationStore'
import { DialogFrame } from './ui/DialogFrame'
import { btn } from './ui/dialogStyles'

/**
 * A record to mix against.
 *
 * Switching between your own mix and a track you trust is how a mix
 * gets finished, but only at the same loudness: louder always sounds
 * better, so comparing at two levels compares the levels. The
 * reference is played instead of the mix, past the master chain and
 * the master fader, with a gain that puts it where the mix sits.
 */

interface ReferenceStatus {
  loaded: boolean
  playing: boolean
  gainDb: number
  lufs: number
  name: string
}

export function ReferencePanel({ onClose }: { onClose: () => void }) {
  const [status, setStatus] = useState<ReferenceStatus>({
    loaded: false, playing: false, gainDb: 0, lufs: -Infinity, name: '',
  })
  const mixLufs = useMeterStore(s => s.master.lufs_i)

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<ReferenceStatus>('get_reference'))
    } catch { /* no engine in the browser preview */ }
  }, [])

  useEffect(() => { void refresh() }, [refresh])

  const pick = useCallback(async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const picked = await open({
        multiple: false,
        filters: [{ name: 'Audio', extensions: ['wav', 'flac', 'mp3', 'ogg', 'aac', 'm4a', 'aiff'] }],
      })
      if (typeof picked !== 'string') return
      const next = await invoke<ReferenceStatus>('load_reference', { path: picked })
      setStatus(next)
      useNotificationStore.getState().push('info', `Reference: ${next.name}`, {
        detail: `${next.lufs.toFixed(1)} LUFS. Match the loudness before you compare.`,
      })
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not load that track', { detail: String(e) })
    }
  }, [])

  const toggle = useCallback(async () => {
    const next = !status.playing
    setStatus(s => ({ ...s, playing: next }))
    try {
      await invoke('set_reference_playing', { playing: next })
    } catch { void refresh() }
  }, [status.playing, refresh])

  const match = useCallback(async () => {
    try {
      const gain = await invoke<number>('match_reference_loudness', { mixLufs })
      setStatus(s => ({ ...s, gainDb: gain }))
      useNotificationStore.getState().push('info', 'Loudness matched', {
        detail: `The reference plays ${gain >= 0 ? '+' : ''}${gain.toFixed(1)} dB, so it sits where your mix does.`,
      })
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not match the loudness', { detail: String(e) })
    }
  }, [mixLufs])

  const setGain = useCallback(async (gainDb: number) => {
    setStatus(s => ({ ...s, gainDb }))
    try {
      await invoke('set_reference_gain', { gainDb })
    } catch { /* the slider already moved */ }
  }, [])

  const clear = useCallback(async () => {
    try {
      await invoke('clear_reference')
      await refresh()
    } catch { /* ignore */ }
  }, [refresh])

  const mixText = Number.isFinite(mixLufs) ? `${(mixLufs as number).toFixed(1)} LUFS` : 'not measured yet'

  return (
    <DialogFrame
      title="Reference track"
      subtitle="Compare at the same loudness, not the same fader"
      onClose={onClose}
      width={520}
    >
      <div style={{ padding: 14, display: 'flex', flexDirection: 'column', gap: 12 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <button onClick={() => { void pick() }} style={btn(true)}>
            {status.loaded ? 'Choose another track…' : 'Choose a track…'}
          </button>
          <span style={{ fontSize: 12.5, color: hw.textSecondary }}>
            {status.loaded ? status.name : 'nothing loaded'}
          </span>
          {status.loaded && (
            <>
              <div style={{ flex: 1 }} />
              <button onClick={() => { void clear() }} style={btn()}>Remove</button>
            </>
          )}
        </div>

        {status.loaded && (
          <>
            <div style={{ display: 'flex', gap: 18, fontSize: 12.5, color: hw.textSecondary }}>
              <span>Reference: <b style={{ color: hw.textPrimary }}>
                {Number.isFinite(status.lufs) ? `${status.lufs.toFixed(1)} LUFS` : '—'}
              </b></span>
              <span>Your mix: <b style={{ color: hw.textPrimary }}>{mixText}</b></span>
            </div>

            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <button
                onClick={() => { void toggle() }}
                style={{
                  padding: '6px 16px', fontSize: 12, fontWeight: 700,
                  borderRadius: hw.radius.sm, border: 'none', cursor: 'pointer',
                  background: status.playing ? hw.accent : 'rgba(255,255,255,0.08)',
                  color: status.playing ? '#fff' : hw.textSecondary,
                  fontFamily: 'inherit',
                }}
              >{status.playing ? 'Hearing the reference' : 'Hearing your mix'}</button>
              <button onClick={() => { void match() }} style={btn()}>Match loudness</button>
            </div>

            <label style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 12.5, color: hw.textSecondary }}>
              Reference gain
              <input
                type="range" min={-24} max={24} step={0.1}
                value={status.gainDb}
                onChange={(e) => { void setGain(Number(e.target.value)) }}
                style={{ flex: 1, accentColor: hw.accent }}
              />
              <span style={{ width: 54, textAlign: 'right', fontVariantNumeric: 'tabular-nums' }}>
                {status.gainDb >= 0 ? '+' : ''}{status.gainDb.toFixed(1)} dB
              </span>
            </label>

            <div style={{ fontSize: 11, color: hw.textFaint, lineHeight: 1.5 }}>
              The reference plays instead of the mix and skips the master chain
              and the master fader, so what you hear is the record itself. It
              follows the playhead, so moving in the song moves in the record.
            </div>
          </>
        )}
      </div>
    </DialogFrame>
  )
}

