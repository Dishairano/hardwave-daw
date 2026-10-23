import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'

interface TakeFormat {
  bits24: boolean
  folder: string | null
}

const buttonStyle = (on = false): React.CSSProperties => ({
  padding: '3px 8px', fontSize: 10, borderRadius: hw.radius.sm, cursor: 'pointer',
  background: on ? hw.accent : 'rgba(255,255,255,0.06)',
  color: on ? '#fff' : hw.textSecondary,
  border: `1px solid ${on ? hw.accent : hw.borderDark}`,
})

/**
 * How recorded takes are saved: 32-bit float, exactly as captured, or
 * 24-bit; and in the song's own Recordings folder or one folder for every
 * take. Takes go to disk while they are played, so the folder also has to
 * have room for the longest take.
 */
export function TakeFormatRow() {
  const [format, setFormat] = useState<TakeFormat | null>(null)
  useEffect(() => {
    invoke<TakeFormat>('get_take_format').then(setFormat).catch(() => setFormat({ bits24: false, folder: null }))
  }, [])
  if (!format) return null

  const save = (next: TakeFormat) => {
    const before = format
    setFormat(next)
    invoke('set_take_format', { format: next }).catch(() => setFormat(before))
  }
  const chooseFolder = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const picked = await open({ directory: true, multiple: false, title: 'Folder for recorded takes' })
      if (typeof picked === 'string') save({ ...format, folder: picked })
    } catch { /* no dialog outside the app */ }
  }

  return (
    <div style={{
      display: 'flex', flexDirection: 'column', gap: 6, marginTop: 8, padding: '8px 12px',
      background: hw.bgPanel, borderRadius: hw.radius.sm, border: `1px solid ${hw.borderDark}`,
    }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <span style={{ fontSize: 11, color: hw.textMuted }}>Takes are saved as</span>
        <div style={{ display: 'flex', gap: 4 }}>
          <button style={buttonStyle(!format.bits24)} onClick={() => save({ ...format, bits24: false })}>32-bit float</button>
          <button style={buttonStyle(format.bits24)} onClick={() => save({ ...format, bits24: true })}>24-bit</button>
        </div>
      </div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 8 }}>
        <span style={{ fontSize: 11, color: hw.textMuted, flexShrink: 0 }}>Takes are saved in</span>
        <span
          title={format.folder ?? undefined}
          style={{ fontSize: 10, color: hw.textSecondary, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', direction: 'rtl', minWidth: 0 }}
        >
          {format.folder ?? "The song's Recordings folder"}
        </span>
        <div style={{ display: 'flex', gap: 4, flexShrink: 0 }}>
          <button style={buttonStyle()} onClick={() => { void chooseFolder() }}>Choose…</button>
          {format.folder && (
            <button style={buttonStyle()} onClick={() => save({ ...format, folder: null })}>With the song</button>
          )}
        </div>
      </div>
      <span style={{ fontSize: 10, color: hw.textFaint }}>
        Takes go to disk while you play, with no length limit: the folder needs room for the longest take.
      </span>
    </div>
  )
}
