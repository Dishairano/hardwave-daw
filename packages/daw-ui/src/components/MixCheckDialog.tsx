import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { hw } from '../theme'

interface Finding { severity: 'problem' | 'warning' | 'info'; title: string; detail: string }
interface Measured {
  durationSecs: number
  peakDb: number
  truePeakDb: number
  lufsIntegrated: number | null
  lufsShortMax: number | null
  crestDb: number
}
interface MixCheck { measured: Measured; findings: Finding[] }

const DOT: Record<Finding['severity'], string> = { problem: '#ef4444', warning: '#f59e0b', info: '#71717a' }

const time = (secs: number) => `${Math.floor(secs / 60)}:${String(Math.floor(secs % 60)).padStart(2, '0')}`
const db = (v: number | null, sign = false) =>
  v == null || v <= -99 ? '-∞' : (sign && v > 0 ? '+' : '') + v.toFixed(1)

/**
 * Tools > Mix check: renders the song as an export would and lists what is
 * worth knowing before it goes out, judged against harder-styles releases.
 */
export function MixCheckDialog({ onClose }: { onClose: () => void }) {
  const [result, setResult] = useState<MixCheck | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [progress, setProgress] = useState(0)
  const [checkedAt, setCheckedAt] = useState<Date | null>(null)

  const run = useCallback(() => {
    setResult(null)
    setError(null)
    setProgress(0)
    invoke<MixCheck>('mix_check')
      .then(r => { setResult(r); setCheckedAt(new Date()) })
      .catch(e => setError(String(e)))
  }, [])

  useEffect(() => {
    let stop: (() => void) | undefined
    listen<number>('mix-check-progress', ev => setProgress(ev.payload)).then(un => { stop = un }).catch(() => {})
    run()
    return () => stop?.()
  }, [run])

  useEffect(() => {
    const escape = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', escape)
    return () => window.removeEventListener('keydown', escape)
  }, [onClose])

  const m = result?.measured
  return (
    <div
      onMouseDown={e => { if (e.target === e.currentTarget) onClose() }}
      style={{ position: 'fixed', inset: 0, zIndex: 9000, background: 'rgba(0,0,0,0.55)', display: 'flex', alignItems: 'flex-start', justifyContent: 'center', paddingTop: 70 }}
    >
      <div style={{ width: 580, maxWidth: 'calc(100vw - 32px)', background: '#121216', border: '1px solid #2a2a33', borderRadius: 10, boxShadow: '0 24px 60px rgba(0,0,0,0.7)', color: '#e4e4e7', fontSize: 13, overflow: 'hidden' }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '12px 16px', borderBottom: '1px solid #23232a', fontWeight: 700 }}>
          Mix check
          <span style={{ fontWeight: 500, color: '#74747e', fontSize: 12 }}>
            {m && checkedAt ? `${time(m.durationSecs)}, rendered at ${checkedAt.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}` : ''}
          </span>
        </div>

        {!result && !error && (
          <div style={{ padding: '28px 16px', color: hw.textMuted }}>
            Rendering the song with its plug-ins… {progress}%
            <div style={{ marginTop: 10, height: 4, background: '#1f1f26', borderRadius: 2 }}>
              <div style={{ width: `${progress}%`, height: '100%', background: hw.accent, borderRadius: 2 }} />
            </div>
          </div>
        )}
        {error && <div style={{ padding: '20px 16px', color: '#f87171' }}>{error}</div>}

        {m && (
          <>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 8, padding: '12px 16px' }}>
              {[
                [db(m.peakDb), 'peak dBFS'],
                [db(m.truePeakDb, true), 'true peak dBTP'],
                [db(m.lufsIntegrated), 'LUFS integrated'],
                [`${m.crestDb.toFixed(1)} dB`, 'crest factor'],
              ].map(([n, l]) => (
                <div key={l} style={{ background: '#0c0c10', border: '1px solid #1f1f26', borderRadius: 6, padding: 8 }}>
                  <div style={{ font: "600 16px 'JetBrains Mono', monospace" }}>{n}</div>
                  <div style={{ color: '#74747e', fontSize: 11 }}>{l}</div>
                </div>
              ))}
            </div>
            {result!.findings.map((f, i) => (
              <div key={i} style={{ display: 'grid', gridTemplateColumns: '10px 1fr', gap: 10, padding: '9px 16px', borderTop: '1px solid #1c1c22' }}>
                <i style={{ width: 8, height: 8, borderRadius: '50%', marginTop: 6, background: DOT[f.severity] }} />
                <div>
                  <div style={{ fontWeight: 600 }}>{f.title}</div>
                  <div style={{ color: '#9a9aa4', fontSize: 12 }}>{f.detail}</div>
                </div>
              </div>
            ))}
          </>
        )}

        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '10px 16px', borderTop: '1px solid #23232a', color: '#74747e', fontSize: 11.5 }}>
          Compared against harder-styles releases
          <div style={{ display: 'flex', gap: 8 }}>
            <button onClick={onClose} style={{ background: 'transparent', color: '#c8c8d0', border: '1px solid #2a2a33', borderRadius: 6, padding: '6px 12px', fontSize: 12, cursor: 'pointer' }}>Close</button>
            <button onClick={run} disabled={!result && !error} style={{ background: '#dc2626', color: '#fff', border: 'none', borderRadius: 6, padding: '6px 12px', fontWeight: 600, fontSize: 12, cursor: 'pointer', opacity: !result && !error ? 0.5 : 1 }}>Check again</button>
          </div>
        </div>
      </div>
    </div>
  )
}
