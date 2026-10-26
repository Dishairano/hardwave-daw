import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  type VelocityCurve,
  type WizardStep,
  useSetupWizardStore,
} from '../stores/setupWizardStore'
import { t, tf, useLanguage } from '../i18n'

/**
 * First-run MIDI Setup Wizard. Five steps walk the user through MIDI
 * device discovery, per-input enable, velocity curve selection, an
 * activity-light test, and the close handoff. Mirrors FL Studio's
 * documented Setup Wizard flow.
 *
 * Backend integration:
 *  - `list_midi_inputs` for the device list
 *  - `open_midi_input` / `close_midi_input` to honour the per-device toggle
 *  - `get_midi_activity` for the activity LED in the test step
 *
 * Velocity curves are stored in `setupWizardStore` only — backend
 * application is queued for the permanent-panel ship. The wizard
 * collects the preference; the audio thread will read it later.
 */

const STEPS: Array<{ id: WizardStep; label: string }> = [
  { id: 'welcome', label: 'Welcome' },
  { id: 'audio', label: 'Audio' },
  { id: 'devices', label: 'MIDI' },
  { id: 'velocity', label: 'Velocity' },
  { id: 'test', label: 'Test' },
  { id: 'done', label: 'Done' },
]

interface MidiActivitySnapshot {
  open_ports: string[]
  ms_since_last_event: number | null
}

export function SetupWizard() {
  const {
    visible,
    step,
    midiMasterEnabled,
    velocityCurves,
    close,
    next,
    back,
    skipForever,
    markComplete,
    setStep,
    setMidiMasterEnabled,
    setVelocityCurve,
  } = useSetupWizardStore()
  useLanguage()

  const [ports, setPorts] = useState<string[]>([])
  const [enabledPorts, setEnabledPorts] = useState<Set<string>>(new Set())
  const [activity, setActivity] = useState<MidiActivitySnapshot | null>(null)
  const [scanning, setScanning] = useState(false)
  const [driver, setDriver] = useState<{ ok: boolean; detail: string } | null>(null)

  const rescan = useCallback(async () => {
    setScanning(true)
    try {
      const list = (await invoke('list_midi_inputs')) as string[]
      setPorts(list)
      // Default: all detected ports get enabled at first sight, unless
      // the user later toggles them off.
      setEnabledPorts(new Set(list))
    } catch (err) {
      console.warn('list_midi_inputs failed:', err)
    } finally {
      setScanning(false)
    }
  }, [])

  // Scan on open + when step changes to devices.
  useEffect(() => {
    if (!visible) return
    void rescan()
    // Whether MIDI works at all is a separate question from whether anything
    // is plugged in, and the wizard used to answer it with a tick drawn
    // without asking.
    invoke<{ ok: boolean; detail: string }>('midi_driver_status')
      .then(setDriver)
      .catch(err => setDriver({ ok: false, detail: String(err) }))
  }, [visible, rescan])

  // Activity polling only while the Test step is showing — avoids
  // hammering the engine when the wizard is in a different state.
  useEffect(() => {
    if (!visible || step !== 'test') return
    const id = setInterval(async () => {
      try {
        const snap = (await invoke('get_midi_activity')) as MidiActivitySnapshot
        setActivity(snap)
      } catch {
        /* ignore — engine may be reloading */
      }
    }, 250)
    return () => clearInterval(id)
  }, [visible, step])

  const toggleDevice = useCallback(
    async (port: string) => {
      const next = new Set(enabledPorts)
      if (next.has(port)) {
        next.delete(port)
        try {
          await invoke('close_midi_input', { portName: port })
        } catch (err) {
          console.warn('close_midi_input failed:', err)
        }
      } else {
        next.add(port)
        try {
          await invoke('open_midi_input', { portName: port })
        } catch (err) {
          console.warn('open_midi_input failed:', err)
        }
      }
      setEnabledPorts(next)
    },
    [enabledPorts],
  )

  if (!visible) return null

  const stepIdx = STEPS.findIndex((s) => s.id === step)

  return (
    <div className="hw-setup-wizard-backdrop" role="dialog" aria-modal="true">
      <div className="hw-setup-wizard">
        <header className="hw-setup-wizard-head">
          <h3>{t('Setup Wizard')}</h3>
          <button type="button" className="hw-setup-wizard-skip" onClick={skipForever}>
            {t("Skip — won't ask again")}
          </button>
        </header>

        <div className="hw-setup-wizard-stepper">
          {STEPS.map((s, i) => (
            <div
              key={s.id}
              className={
                'hw-setup-wizard-step' +
                (i < stepIdx ? ' is-done' : '') +
                (i === stepIdx ? ' is-active' : '')
              }
              onClick={() => i <= stepIdx && setStep(s.id)}
            >
              {t(s.label)}
            </div>
          ))}
        </div>

        <div className="hw-setup-wizard-body">
          {step === 'welcome' && (
            <WelcomeStep ports={ports} scanning={scanning} rescan={rescan} driver={driver} />
          )}
          {step === 'audio' && <AudioStep />}
          {step === 'devices' && (
            <DevicesStep
              ports={ports}
              enabled={enabledPorts}
              onToggle={toggleDevice}
              masterEnabled={midiMasterEnabled}
              onMasterChange={setMidiMasterEnabled}
              rescan={rescan}
              scanning={scanning}
            />
          )}
          {step === 'velocity' && (
            <VelocityStep
              ports={ports}
              curves={velocityCurves}
              onChange={setVelocityCurve}
            />
          )}
          {step === 'test' && <TestStep activity={activity} />}
          {step === 'done' && <DoneStep />}
        </div>

        <footer className="hw-setup-wizard-foot">
          <span className="hw-setup-wizard-meta">
            {tf('Step {n} of {total}', { n: stepIdx + 1, total: STEPS.length })}
          </span>
          <div className="hw-setup-wizard-actions">
            {stepIdx > 0 && step !== 'done' && (
              <button type="button" className="hw-btn" onClick={back}>
                {t('Back')}
              </button>
            )}
            {(step === 'welcome' || step === 'devices') && (
              <button
                type="button"
                className="hw-btn hw-btn-ghost"
                onClick={() => void rescan()}
                disabled={scanning}
              >
                {scanning ? t('Scanning…') : t('Rescan')}
              </button>
            )}
            {step === 'done' ? (
              <button type="button" className="hw-btn hw-btn-primary" onClick={markComplete}>
                {t('Open Hardwave')}
              </button>
            ) : (
              <button
                type="button"
                className="hw-btn hw-btn-primary"
                onClick={() => (step === 'test' ? next() : next())}
              >
                {t('Continue')}
              </button>
            )}
            {step === 'done' && (
              <button type="button" className="hw-btn hw-btn-ghost" onClick={close}>
                {t('Close')}
              </button>
            )}
          </div>
        </footer>
      </div>
    </div>
  )
}

// ── Step bodies ────────────────────────────────────────────────────────────

interface WizardAudioDevice {
  name: string
  is_default: boolean
  sample_rates: number[]
  max_channels: number
}

const BUFFER_CHOICES = [128, 256, 512, 1024]

/** Audio output setup — the one choice that ruins a first session when
 * wrong (system speakers instead of the interface). Reuses the same
 * commands as File → Audio settings; applying here writes the same
 * persisted prefs, so the two stay in sync. */
function AudioStep() {
  useLanguage()
  const [devices, setDevices] = useState<WizardAudioDevice[]>([])
  const [device, setDevice] = useState<string | null>(null)
  const [sampleRate, setSampleRate] = useState(48000)
  const [bufferSize, setBufferSize] = useState(512)
  const [applied, setApplied] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    Promise.all([
      invoke<WizardAudioDevice[]>('get_audio_devices'),
      invoke<{ device: string | null; sample_rate: number; buffer_size: number }>('get_audio_config'),
    ])
      .then(([devs, cfg]) => {
        setDevices(devs)
        setDevice(cfg.device ?? devs.find((d) => d.is_default)?.name ?? null)
        setSampleRate(cfg.sample_rate)
        setBufferSize(cfg.buffer_size)
      })
      .catch((e) => setError(String(e)))
  }, [])

  const latencyMs = ((bufferSize / sampleRate) * 1000).toFixed(1)
  // The menu path sits in the sentence wherever the language puts it.
  const [changeBefore, changeAfter] = t('You can change this any time under {where}.').split('{where}')
  const selected = devices.find((d) => d.name === device)
  const rates = selected?.sample_rates?.length ? selected.sample_rates : [44100, 48000]

  const apply = async () => {
    setBusy(true)
    setError(null)
    try {
      await invoke('set_audio_config', { device, sampleRate, bufferSize })
      setApplied(tf('{device} @ {rate} Hz / {buffer} samples', { device: device ?? t('Default device'), rate: sampleRate, buffer: bufferSize }))
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <h4>{t('Where should Hardwave play audio?')}</h4>
      <p className="hw-setup-wizard-muted">
        {t('Pick your audio interface (not the system speakers, if you have one) and a buffer size.')}{' '}
        {changeBefore}<strong>{t('File → Audio settings')}</strong>{changeAfter}
      </p>

      <div className="hw-setup-wizard-audio-grid">
        <label>
          {t('Output device')}
          <select
            value={device ?? ''}
            onChange={(e) => setDevice(e.target.value || null)}
          >
            {devices.map((d) => (
              <option key={d.name} value={d.name}>
                {d.is_default ? tf('{name} (system default)', { name: d.name }) : d.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t('Sample rate')}
          <select value={sampleRate} onChange={(e) => setSampleRate(Number(e.target.value))}>
            {rates.map((r) => (
              <option key={r} value={r}>{r} Hz</option>
            ))}
          </select>
        </label>
        <label>
          {t('Buffer size')}
          <select value={bufferSize} onChange={(e) => setBufferSize(Number(e.target.value))}>
            {BUFFER_CHOICES.map((b) => (
              <option key={b} value={b}>{tf('{n} samples', { n: b })}</option>
            ))}
          </select>
        </label>
      </div>

      <p className="hw-setup-wizard-muted">
        <strong>{tf('Latency ≈ {ms} ms.', { ms: latencyMs })}</strong>{' '}
        {t('512 samples is a safe default; drop to 256/128 for recording if your machine keeps up (crackles mean the buffer is too small).')}
      </p>

      <div className="hw-setup-wizard-actions" style={{ justifyContent: 'flex-start' }}>
        <button type="button" className="hw-btn hw-btn-primary" onClick={() => void apply()} disabled={busy || devices.length === 0}>
          {busy ? t('Applying…') : t('Apply & test')}
        </button>
        {applied && !error && (
          <span className="hw-setup-wizard-muted" style={{ alignSelf: 'center' }}>
            ✓ {tf('Active: {setting}', { setting: applied })}
          </span>
        )}
      </div>
      {error && (
        <p className="hw-setup-wizard-muted" style={{ color: 'var(--brand, #e94560)' }}>
          {tf('Could not apply: {error}', { error })}
        </p>
      )}
    </>
  )
}

function WelcomeStep({
  ports,
  scanning,
  driver,
}: {
  ports: string[]
  scanning: boolean
  rescan: () => void
  /** null while the check is still running. */
  driver: { ok: boolean; detail: string } | null
}) {
  useLanguage()
  // The button's name sits in the sentence wherever the language puts it.
  const [rescanBefore, rescanAfter] = t('Hardwave detected the following ports. If your controller is missing, plug it in now — click {rescan} when ready.').split('{rescan}')
  return (
    <>
      <h4>{t('Connect your MIDI controller')}</h4>
      <p>
        {rescanBefore}<strong>{t('Rescan')}</strong>{rescanAfter}
      </p>
      <div className="hw-setup-wizard-checklist">
        <div className="hw-setup-wizard-check">
          <span className={`hw-check-icon ${ports.length > 0 ? 'ok' : 'warn'}`}>
            {ports.length > 0 ? '✓' : '!'}
          </span>
          <div>
            <b>{ports.length === 1 ? tf('{n} MIDI input found', { n: ports.length }) : tf('{n} MIDI inputs found', { n: ports.length })}</b>
            <div className="hw-setup-wizard-muted">
              {ports.length === 0 && !scanning ? (
                <>
                  {t('No ports detected. Connect a controller and hit Rescan, or skip if you only plan to use the typing keyboard / on-screen Touch Controller.')}
                </>
              ) : (
                ports.join(' · ')
              )}
            </div>
          </div>
        </div>
        <div className="hw-setup-wizard-check">
          <span className={`hw-check-icon ${driver == null ? 'warn' : driver.ok ? 'ok' : 'warn'}`}>
            {driver == null ? '…' : driver.ok ? '✓' : '!'}
          </span>
          <div>
            <b>
              {driver == null
                ? t('Checking the MIDI driver')
                : driver.ok
                  ? t('MIDI driver working')
                  : t('MIDI driver not available')}
            </b>
            <div className="hw-setup-wizard-muted">
              {driver == null
                ? t('One moment.')
                : driver.ok
                  ? t('The system MIDI service answered, so controllers can be opened.')
                  : tf('Controllers cannot be opened on this machine: {detail}', { detail: driver.detail })}
            </div>
          </div>
        </div>
      </div>
      <p className="hw-setup-wizard-muted">
        {t('Right after this step you can enable or disable each input individually, and set how hard each one has to be hit.')}
      </p>
    </>
  )
}

function DevicesStep({
  ports,
  enabled,
  onToggle,
  masterEnabled,
  onMasterChange,
  rescan,
  scanning,
}: {
  ports: string[]
  enabled: Set<string>
  onToggle: (port: string) => void
  masterEnabled: boolean
  onMasterChange: (v: boolean) => void
  rescan: () => void
  scanning: boolean
}) {
  useLanguage()
  return (
    <>
      <h4>{t('Enable inputs')}</h4>
      <p className="hw-setup-wizard-muted">
        {t("Each row maps to a connected MIDI device. Toggle off any port you don't want feeding Hardwave — useful for virtual loopback ports or noisy hardware.")}
      </p>

      <label className="hw-setup-wizard-master">
        <span>
          <strong>{t('Master — Enable MIDI remote control')}</strong>
          <small>{t('When off, no MIDI input reaches the audio thread.')}</small>
        </span>
        <input
          type="checkbox"
          checked={masterEnabled}
          onChange={(e) => onMasterChange(e.target.checked)}
        />
      </label>

      <div className="hw-setup-wizard-devices">
        {ports.length === 0 && (
          <div className="hw-setup-wizard-empty">
            {t('No ports detected.')} {scanning ? t('Scanning…') : (
              <a onClick={rescan} role="button">{t('Rescan')}</a>
            )}
          </div>
        )}
        {ports.map((port) => {
          const isOn = enabled.has(port)
          return (
            <div key={port} className={`hw-setup-wizard-device${isOn ? '' : ' is-off'}`}>
              <span className="hw-setup-wizard-led" aria-hidden />
              <div className="hw-setup-wizard-device-name">
                <strong>{port}</strong>
                <small>{isOn ? t('Listening') : t('Disabled')}</small>
              </div>
              <button
                type="button"
                className={`hw-setup-wizard-toggle${isOn ? ' is-on' : ''}`}
                onClick={() => onToggle(port)}
                aria-label={tf('Toggle {name}', { name: port })}
              />
            </div>
          )
        })}
      </div>
    </>
  )
}

const CURVES: Array<{ id: VelocityCurve; label: string; desc: string }> = [
  { id: 'linear', label: 'Linear', desc: 'Pass through' },
  { id: 'soft', label: 'Soft', desc: 'Lift light hits' },
  { id: 'hard', label: 'Hard', desc: 'Hold light hits down' },
  { id: 's-curve', label: 'S-curve', desc: 'More dynamics' },
]

function VelocityStep({
  ports,
  curves,
  onChange,
}: {
  ports: string[]
  curves: Record<string, VelocityCurve>
  onChange: (port: string, curve: VelocityCurve) => void
}) {
  useLanguage()
  const [activePort, setActivePort] = useState<string | null>(ports[0] ?? null)
  useEffect(() => {
    if (!activePort && ports.length > 0) setActivePort(ports[0])
  }, [ports, activePort])

  if (ports.length === 0) {
    return (
      <>
        <h4>{t('Velocity curve')}</h4>
        <p className="hw-setup-wizard-muted">
          {t('No hardware inputs to calibrate. The typing keyboard + Touch Controller already use a fixed default velocity (or position-based when enabled in Touch settings). Skip ahead to the activity test.')}
        </p>
      </>
    )
  }

  const currentCurve = activePort ? curves[activePort] ?? 'linear' : 'linear'

  return (
    <>
      <h4>{t('Velocity curve')}</h4>
      <p className="hw-setup-wizard-muted">
        {t("Pick a preset that matches your controller's velocity feel. Curves are per-input, so a light-action Novation can run Soft while a heavier MPC stays Linear.")}
      </p>

      {ports.length > 1 && (
        <div className="hw-setup-wizard-port-tabs">
          {ports.map((p) => (
            <button
              key={p}
              type="button"
              className={`hw-setup-wizard-port-tab${p === activePort ? ' is-active' : ''}`}
              onClick={() => setActivePort(p)}
            >
              {p}
            </button>
          ))}
        </div>
      )}

      <div className="hw-setup-wizard-curve-row">
        <div className="hw-setup-wizard-curve-presets">
          {CURVES.map((c) => (
            <button
              key={c.id}
              type="button"
              className={`hw-setup-wizard-curve-preset${
                c.id === currentCurve ? ' is-active' : ''
              }`}
              onClick={() => activePort && onChange(activePort, c.id)}
            >
              <strong>{t(c.label)}</strong>
              <small>{t(c.desc)}</small>
            </button>
          ))}
        </div>
        <div className="hw-setup-wizard-curve-canvas">
          <svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden>
            <line
              x1="0"
              y1="100"
              x2="100"
              y2="0"
              stroke="rgba(255,255,255,0.1)"
              strokeWidth="0.5"
              strokeDasharray="2 2"
            />
            <path d={curvePath(currentCurve)} stroke="var(--brand, #e94560)" strokeWidth="2" fill="none" />
          </svg>
        </div>
      </div>

      <p className="hw-setup-wizard-muted">
        {t('The curve applies to this port as soon as you pick it, and is remembered for the next time you start Hardwave. Every curve leaves your softest and hardest hits where they are and only moves what is in between, so nothing you can play becomes unreachable.')}
      </p>
    </>
  )
}

function curvePath(curve: VelocityCurve): string {
  // Render a representative curve as an SVG path. y-axis is inverted
  // (0,100 = bottom-left). All curves end at (100, 0) — top-right.
  // Shaped to match what the backend actually does: a square root for soft,
  // a square for hard, smoothstep for the S. A drawing that does not match
  // the sound is the same lie as a control that does nothing.
  switch (curve) {
    case 'soft':
      return 'M 0 100 C 15 45, 55 12, 100 0'
    case 'hard':
      return 'M 0 100 C 45 88, 78 55, 100 0'
    case 's-curve':
      return 'M 0 100 C 33 100, 67 0, 100 0'
    case 'linear':
    default:
      return 'M 0 100 L 100 0'
  }
}

function TestStep({ activity }: { activity: MidiActivitySnapshot | null }) {
  useLanguage()
  const fresh = activity != null && activity.ms_since_last_event != null && activity.ms_since_last_event < 800
  return (
    <>
      <h4>{t('Play a note — Hardwave is listening')}</h4>
      <div className="hw-setup-wizard-activity">
        <div className={`hw-setup-wizard-led-large${fresh ? ' is-fresh' : ''}`} aria-hidden />
        <div>
          <h5>
            {fresh
              ? t('MIDI activity detected — you are connected')
              : t('Waiting for input…')}
          </h5>
          <p className="hw-setup-wizard-muted">
            {t("The big LED pulses every time the engine receives a MIDI message. Notes play through Hardwave's default sine voice so you hear what you're playing.")}
          </p>
          {activity && (
            <p className="hw-setup-wizard-muted" style={{ fontSize: 11 }}>
              {activity.open_ports.length === 1
                ? tf('{n} port open', { n: activity.open_ports.length })
                : tf('{n} ports open', { n: activity.open_ports.length })}
              {' · '}
              {activity.ms_since_last_event == null
                ? t('no events yet')
                : tf('last event {ms} ms ago', { ms: activity.ms_since_last_event })}
            </p>
          )}
        </div>
      </div>
      <p className="hw-setup-wizard-muted">
        {t('No activity? Click Back to recheck your devices, or run Rescan from the Tools menu later.')}
      </p>
    </>
  )
}

function DoneStep() {
  useLanguage()
  return (
    <>
      <h4>{t("You're set")} 🎹</h4>
      <p>{t('Hardwave will remember your choices for the next time you open the app. To change them later:')}</p>
      <div className="hw-setup-wizard-checklist">
        <div className="hw-setup-wizard-check">
          <span className="hw-check-icon arrow">→</span>
          <div>
            <b>{t('Tools → MIDI mappings…')}</b>
            <div className="hw-setup-wizard-muted">{t('Per-CC mappings + the Learn flow')}</div>
          </div>
        </div>
        <div className="hw-setup-wizard-check">
          <span className="hw-check-icon arrow">→</span>
          <div>
            <b>{t('File → Audio settings…')}</b>
            <div className="hw-setup-wizard-muted">
              {t('Input / output ports + velocity curves (permanent panel)')}
            </div>
          </div>
        </div>
        <div className="hw-setup-wizard-check">
          <span className="hw-check-icon arrow">→</span>
          <div>
            <b>{t('Help → Re-run setup wizard')}</b>
            <div className="hw-setup-wizard-muted">{t('Walks through this flow again')}</div>
          </div>
        </div>
      </div>
    </>
  )
}
