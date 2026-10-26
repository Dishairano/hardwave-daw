import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'
import { t, useLanguage } from '../../i18n'

/**
 * How late the plug-ins make the song play, on top of the output latency.
 *
 * The engine adds it up for delay compensation, and nothing showed it, so a
 * lookahead limiter on the master that put 200 ms between a key press and
 * the sound looked like a slow audio device.
 */
export function PluginLatencyRow() {
  useLanguage()
  const [latency, setLatency] = useState<{ ms: number; pdcEnabled: boolean } | null>(null)
  useEffect(() => {
    let alive = true
    const read = () => {
      invoke<{ samples: number; ms: number; pdcEnabled: boolean }>('get_graph_latency')
        .then(v => { if (alive) setLatency(v) })
        .catch(() => {})
    }
    read()
    // Plug-ins come and go while the settings are open.
    const timer = setInterval(read, 2000)
    return () => { alive = false; clearInterval(timer) }
  }, [])
  if (!latency) return null
  return (
    <div
      title={latency.pdcEnabled
        ? t('The slowest plug-in chain: the other tracks wait this long so everything stays in time')
        : t('Delay compensation is off (Options menu), so no track waits for its plug-ins')}
      style={{
        display: 'flex', justifyContent: 'space-between', alignItems: 'center',
        marginTop: 6, padding: '8px 12px',
        background: hw.bgPanel, borderRadius: hw.radius.sm,
        border: `1px solid ${hw.borderDark}`,
      }}
    >
      <span style={{ fontSize: 11, color: hw.textMuted }}>{t('Plug-in delay')}</span>
      <span style={{ fontSize: 12, fontWeight: 600, color: hw.textPrimary, fontFamily: "'Consolas', monospace" }}>
        {latency.pdcEnabled ? `${latency.ms.toFixed(1)} ms` : t('compensation off')}
      </span>
    </div>
  )
}
