import { useEffect, useState } from 'react'
import { getVersion } from '@tauri-apps/api/app'
import { HwLogo } from './HwLogo'
import { hw } from '../theme'

interface AboutDialogProps {
  onClose: () => void
}

export function AboutDialog({ onClose }: AboutDialogProps) {
  const [version, setVersion] = useState<string>('')
  const [tauriVersion, setTauriVersion] = useState<string>('')

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion('dev'))
    import('@tauri-apps/api/app')
      .then(m => m.getTauriVersion().then(setTauriVersion))
      .catch(() => setTauriVersion(''))
  }, [])

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose])

  return (
    <div style={{
      position: 'fixed', inset: 0, zIndex: 9999,
      display: 'flex', alignItems: 'center', justifyContent: 'center',
    }}>
      <div
        style={{
          position: 'absolute', inset: 0,
          background: 'rgba(0,0,0,0.75)',
          backdropFilter: 'blur(8px)',
        }}
        onClick={onClose}
      />
      <div style={{
        position: 'relative',
        width: '100%', maxWidth: 420,
        margin: '0 16px',
        background: hw.bgSurface,
        border: `1px solid ${hw.border}`,
        borderRadius: 14,
        boxShadow: '0 25px 50px rgba(0,0,0,0.6), 0 0 80px rgba(220,38,38,0.08)',
        overflow: 'hidden',
      }}>
        <div style={{
          height: 2,
          background: `linear-gradient(90deg, ${hw.secondary}, ${hw.accent}, ${hw.secondary})`,
        }} />

        <div style={{ padding: '24px 24px 8px', display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
          <HwLogo size={80} />
          <h2 style={{ fontSize: 18, fontWeight: 700, color: hw.textPrimary, margin: '14px 0 2px' }}>
            Hardwave DAW
          </h2>
          <div style={{ fontSize: 11, color: hw.accent, fontFamily: "'Consolas', monospace", fontWeight: 600 }}>
            v{version || '...'}
          </div>
          <div style={{ fontSize: 11, color: hw.textMuted, marginTop: 12, textAlign: 'center', lineHeight: 1.5 }}>
            A modern, open digital audio workstation
            <br />
            built for producers who care about the craft.
          </div>
        </div>

        <div style={{
          margin: '18px 24px 0',
          padding: 12,
          background: 'rgba(255,255,255,0.02)',
          border: `1px solid ${hw.border}`,
          borderRadius: 10,
        }}>
          <Row label="Version" value={version || '—'} />
          {tauriVersion && <Row label="Tauri" value={tauriVersion} />}
          <Row label="Engine" value="Rust + nih-plug" />
          <Row label="License" value="Proprietary" />
        </div>

        <div style={{
          padding: '12px 24px 0',
          fontSize: 10,
          color: hw.textFaint,
          textAlign: 'center',
          lineHeight: 1.6,
        }}>
          © {new Date().getFullYear()} Hardwave Studios. All rights reserved.
        </div>

        <div style={{
          display: 'flex', justifyContent: 'center',
          padding: '16px 24px 20px',
        }}>
          <button
            onClick={onClose}
            style={{
              minWidth: 120, padding: '8px 20px', borderRadius: 10,
              background: `linear-gradient(135deg, ${hw.secondary}, ${hw.accent})`,
              border: 'none',
              fontSize: 12, fontWeight: 700, color: '#FFF',
              boxShadow: `0 4px 16px ${hw.accentGlow}`,
              cursor: 'default',
            }}
            onMouseEnter={e => { e.currentTarget.style.background = hw.accentLight }}
            onMouseLeave={e => {
              e.currentTarget.style.background = `linear-gradient(135deg, ${hw.secondary}, ${hw.accent})`
            }}
          >
            Close
          </button>
        </div>
      </div>
    </div>
  )
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div style={{
      display: 'flex', justifyContent: 'space-between',
      fontSize: 11, padding: '3px 0',
    }}>
      <span style={{ color: hw.textFaint }}>{label}</span>
      <span style={{ color: hw.textSecondary, fontFamily: "'Consolas', monospace" }}>{value}</span>
    </div>
  )
}
