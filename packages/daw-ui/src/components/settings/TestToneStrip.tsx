import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { t, tf, useLanguage } from '../../i18n'

/**
 * The test tone strip: which device is live, said while a soft tone plays on
 * it, left then right.
 *
 * Apply in Settings > Audio switched the device in silence, so someone with
 * an interface and laptop speakers could not tell which one was live until a
 * song played. The tone is made in the engine and goes straight to the
 * device (see the engine's test_tone); this strip names the device and the
 * output pair, with a meter per side. Settings > Audio and the first run's
 * Sound step show the same strip.
 */

/** What `play_test_tone` says about the device it played on. */
export interface ToneDevice {
  device: string
  /** The output pair, as the interface numbers it: "1/2". */
  outputs: string
  sampleRate: number
  bufferSize: number
}

export type ToneState =
  | { kind: 'idle' }
  | { kind: 'played'; on: ToneDevice; at: Date }
  | { kind: 'failed'; reason: string }

interface ToneReading {
  playing: boolean
  side: 'idle' | 'left' | 'right'
  peakLeft: number
  peakRight: number
}

/** A meter's width for a linear peak: -48 dBFS is empty, 0 dBFS is full. */
export function meterPercent(peak: number): number {
  if (!(peak > 0)) return 0
  const db = 20 * Math.log10(peak)
  return Math.max(0, Math.min(100, ((db + 48) / 48) * 100))
}

/** Play the tone, and read its meters while it sounds. */
export function useTestTone() {
  const [state, setState] = useState<ToneState>({ kind: 'idle' })
  const [meters, setMeters] = useState({ left: 0, right: 0 })
  const [sounding, setSounding] = useState(false)
  const startedAt = useRef(0)

  const play = useCallback(async (): Promise<boolean> => {
    try {
      const on = await invoke<ToneDevice>('play_test_tone')
      setState({ kind: 'played', on, at: new Date() })
      startedAt.current = Date.now()
      setSounding(true)
      return true
    } catch (e) {
      setState({ kind: 'failed', reason: String(e) })
      return false
    }
  }, [])

  /** Say the device did not open, with the reason, instead of playing. */
  const failed = useCallback((reason: string) => setState({ kind: 'failed', reason }), [])

  useEffect(() => {
    if (!sounding) return
    let cancelled = false
    let left = 0
    let right = 0
    const id = window.setInterval(async () => {
      let reading: ToneReading | null = null
      try { reading = await invoke<ToneReading>('get_test_tone') } catch { /* engine restarting */ }
      if (cancelled) return
      // A little fall-back on the screen, so the meter moves like one.
      left = Math.max(reading?.peakLeft ?? 0, left * 0.7)
      right = Math.max(reading?.peakRight ?? 0, right * 0.7)
      setMeters({ left, right })
      // Done once the engine says so (or it never answers), and the
      // meters have fallen back.
      const elapsed = Date.now() - startedAt.current
      const over = elapsed > 2000 || (elapsed > 300 && reading !== null && !reading.playing)
      if (over && left < 0.001 && right < 0.001) setSounding(false)
    }, 50)
    return () => { cancelled = true; window.clearInterval(id) }
  }, [sounding])

  return { state, meters, sounding, play, failed }
}

interface StripProps {
  state: ToneState
  meters: { left: number; right: number }
  /** The last part of the second line: "applied 12:04", or the rate and
   *  buffer on the first run. */
  detail: (on: ToneDevice, at: Date) => string
  onPlayAgain: () => void
  /** Retry after the device did not open. */
  onRetry: () => void
}

const GREEN = '#3ed07a'

export function TestToneStrip({ state, meters, detail, onPlayAgain, onRetry }: StripProps) {
  useLanguage()
  const [help, setHelp] = useState(false)
  if (state.kind === 'idle') return null

  if (state.kind === 'failed') {
    return (
      <div data-testid="test-tone-strip" role="status" style={{ ...box, borderColor: 'rgba(239,68,68,0.5)', background: 'rgba(239,68,68,0.07)' }}>
        <span style={{ ...icon, background: 'rgba(239,68,68,0.18)', color: '#ef4444' }} aria-hidden>!</span>
        <div style={{ flex: 1, minWidth: 0 }}>
          <b style={title}>{t('The sound card did not open')}</b>
          <small style={sub}>{state.reason}</small>
          <div style={buttons}>
            <button type="button" style={mini} onClick={onRetry}>{t('Retry')}</button>
          </div>
        </div>
      </div>
    )
  }

  const { on, at } = state
  return (
    <div data-testid="test-tone-strip" role="status" style={box}>
      <span style={icon} aria-hidden>
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
          <path d="M2 6h3l4-3v10l-4-3H2z" />
          <path d="M11.5 5.5a3.5 3.5 0 0 1 0 5M13.5 3.5a6.5 6.5 0 0 1 0 9" />
        </svg>
      </span>
      <div style={{ flex: 1, minWidth: 0 }}>
        <b style={title}>{tf('Test tone on {device}', { device: on.device })}</b>
        <small style={sub}>
          {tf('Outputs {pair}', { pair: on.outputs })} · {t('left, then right')} · {detail(on, at)}
        </small>
        <div style={buttons}>
          <button type="button" style={mini} onClick={onPlayAgain}>{t('Play again')}</button>
          <button type="button" style={mini} aria-expanded={help} onClick={() => setHelp(h => !h)}>{t('I hear nothing')}</button>
        </div>
        {help && (
          <ul data-testid="test-tone-help" style={{ margin: '10px 0 0', paddingLeft: 18, fontSize: 11.5, color: '#d4d4d8', lineHeight: 1.6 }}>
            <li>{t("Turn up the interface's volume knob, and check its monitors or headphones are on.")}</li>
            <li>{t('Pick another output device, or another output pair, and listen again.')}</li>
            <li>{t('Another program may hold the device: close it, or turn off exclusive mode for this device in the Windows sound settings.')}</li>
          </ul>
        )}
      </div>
      <div style={{ width: 170, display: 'flex', flexDirection: 'column', gap: 5, font: "600 9.5px/1 'JetBrains Mono', monospace", color: '#a1a1a6' }}>
        <Meter label="L" peak={meters.left} />
        <Meter label="R" peak={meters.right} />
      </div>
    </div>
  )
}

function Meter({ label, peak }: { label: string; peak: number }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
      {label}
      <span
        data-testid={`test-tone-meter-${label}`}
        style={{ height: 8, borderRadius: 3, background: '#1d1d23', overflow: 'hidden', position: 'relative', flex: 1 }}
      >
        <i style={{
          position: 'absolute', left: 0, top: 0, bottom: 0, width: `${meterPercent(peak)}%`,
          background: 'linear-gradient(90deg, #16a34a, #84cc16 75%, #facc15)',
        }} />
      </span>
    </div>
  )
}

const box: React.CSSProperties = {
  border: `1px solid rgba(62,208,122,0.45)`, background: 'rgba(62,208,122,0.07)', borderRadius: 8,
  padding: '11px 12px', display: 'flex', gap: 12, alignItems: 'flex-start',
}
const icon: React.CSSProperties = {
  width: 30, height: 30, borderRadius: '50%', background: 'rgba(62,208,122,0.18)', color: GREEN,
  display: 'grid', placeItems: 'center', flex: 'none', fontWeight: 700,
}
const title: React.CSSProperties = { fontSize: 12.5, color: '#fff', display: 'block' }
const sub: React.CSSProperties = { fontSize: 11, color: '#b6b6c2', display: 'block', overflowWrap: 'anywhere' }
const buttons: React.CSSProperties = { display: 'flex', gap: 6, marginTop: 8 }
const mini: React.CSSProperties = {
  padding: '4px 11px', borderRadius: 6, fontSize: 11, fontWeight: 600,
  border: '1px solid rgba(255,255,255,0.14)', color: '#e8e8f0', background: 'transparent',
  whiteSpace: 'nowrap', cursor: 'pointer', fontFamily: 'inherit',
}
