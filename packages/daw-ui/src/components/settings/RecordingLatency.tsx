import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'
import { useAudioPrefsStore } from '../../stores/audioPrefsStore'

interface RecordingLatencyInfo {
  outputMs: number
  inputMs: number
  queueMs: number
  offsetMs: number
  totalMs: number
  inputRunning: boolean
}

/**
 * How far each recorded take is moved back so it lands where it was played,
 * with the manual offset for interfaces that under-report their latency.
 * The figure is measured from the running streams, so it is re-read while
 * the panel is open.
 */
export function RecordingLatencyRow() {
  const offset = useAudioPrefsStore(s => s.recordOffsetMs)
  const setOffset = useAudioPrefsStore(s => s.setRecordOffsetMs)
  const [info, setInfo] = useState<RecordingLatencyInfo | null>(null)
  const [draft, setDraft] = useState(String(offset))

  useEffect(() => { setDraft(String(offset)) }, [offset])

  useEffect(() => {
    let alive = true
    const read = () => {
      invoke<RecordingLatencyInfo>('get_recording_latency')
        .then(i => { if (alive) setInfo(i) })
        .catch(() => { /* engine not running */ })
    }
    read()
    const id = window.setInterval(read, 1000)
    return () => { alive = false; window.clearInterval(id) }
  }, [])

  const commit = () => {
    const n = Number(draft)
    if (Number.isFinite(n)) setOffset(n)
    else setDraft(String(offset))
  }

  const f = (ms: number) => ms.toFixed(1)
  const measured = info ? info.outputMs + info.inputMs + info.queueMs : 0

  return (
    <div style={{
      marginTop: 8, padding: '8px 12px',
      background: hw.bgPanel, borderRadius: hw.radius.sm,
      border: `1px solid ${hw.borderDark}`,
    }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 12 }}>
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 11, color: hw.textMuted }}>Recording compensation</div>
          <div style={{ fontSize: 10, color: hw.textFaint, marginTop: 2, lineHeight: 1.4 }}>
            Takes are moved back by this much so they land where you played them.
          </div>
        </div>
        <span style={{ fontSize: 12, fontWeight: 600, color: hw.textPrimary, fontFamily: "'Consolas', monospace", whiteSpace: 'nowrap' }}>
          {info ? `${f(info.totalMs)} ms` : '…'}
        </span>
      </div>
      {info && (
        <div style={{ fontSize: 10, color: hw.textFaint, marginTop: 6, fontFamily: "'Consolas', monospace" }}>
          measured {f(measured)} ms = output {f(info.outputMs)} + input {info.inputRunning ? f(info.inputMs) : '?'} + queue {f(info.queueMs)}
          {!info.inputRunning && ' (arm a track to measure the input)'}
        </div>
      )}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 8 }}>
        <label htmlFor="rec-offset" style={{ fontSize: 11, color: hw.textMuted, flex: 1 }}>
          Manual offset (ms)
          <span style={{ display: 'block', fontSize: 10, color: hw.textFaint }}>
            If takes still sound late, raise it; if early, lower it.
          </span>
        </label>
        <input
          id="rec-offset"
          type="number"
          min={-500}
          max={500}
          step={1}
          value={draft}
          onChange={e => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={e => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur() }}
          style={{
            width: 72, padding: '4px 6px', fontSize: 12, textAlign: 'right',
            background: hw.bgInput, color: hw.textPrimary,
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
          }}
        />
      </div>
    </div>
  )
}
