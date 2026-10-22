import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'

/**
 * Let go of the audio device while another program is in front.
 *
 * An idle DAW holding the sound card made FL Studio hang when FL reset its
 * own device on the same card. On by default; off for someone who wants to
 * hear a MIDI keyboard through the DAW while working in another program.
 */
export function ReleaseInBackgroundRow() {
  const [on, setOn] = useState<boolean | null>(null)
  useEffect(() => {
    invoke<boolean>('get_release_device_in_background').then(setOn).catch(() => setOn(true))
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
        <span style={{ fontSize: 11, color: hw.textMuted }}>Free the sound card in the background</span>
        <span style={{ fontSize: 10, color: hw.textFaint }}>
          When the song is stopped and another program is in front, so FL Studio or another DAW can use the same device
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
