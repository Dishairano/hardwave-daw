import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'
import { t, useLanguage } from '../../i18n'

/**
 * Let go of the audio device while another program is in front, for ASIO
 * drivers that serve one program at a time. Off by default, so a MIDI
 * keyboard played through the DAW keeps sounding while another program is in
 * front.
 */
export function ReleaseInBackgroundRow() {
  useLanguage()
  const [on, setOn] = useState<boolean | null>(null)
  useEffect(() => {
    invoke<boolean>('get_release_device_in_background').then(setOn).catch(() => setOn(false))
  }, [])
  if (on == null) return null
  const toggle = () => {
    const next = !on
    setOn(next)
    invoke('set_release_device_in_background', { enabled: next }).catch(() => setOn(!next))
  }
  return (
    <div style={{
      display: 'flex', justifyContent: 'space-between', alignItems: 'center',
      marginTop: 8, padding: '8px 12px',
      background: hw.bgPanel, borderRadius: hw.radius.sm,
      border: `1px solid ${hw.borderDark}`,
    }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
        <span style={{ fontSize: 11, color: hw.textMuted }}>{t('Free the sound card in the background')}</span>
        <span style={{ fontSize: 10, color: hw.textFaint }}>
          {t('When the song is stopped and another program is in front, so another program can use an ASIO device that takes one program at a time')}
        </span>
      </div>
      <button
        onClick={toggle}
        aria-pressed={on}
        style={{
          width: 38, height: 20, flexShrink: 0,
          borderRadius: 10, border: 'none',
          background: on ? hw.accent : 'rgba(255,255,255,0.08)',
          cursor: 'pointer', position: 'relative', transition: 'background 150ms ease',
        }}
      >
        <span style={{
          position: 'absolute', top: 2, left: on ? 20 : 2,
          width: 16, height: 16, borderRadius: '50%',
          background: '#fff', transition: 'left 150ms ease',
        }} />
      </button>
    </div>
  )
}
