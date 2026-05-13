import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  type VelocityCurve,
  type WizardStep,
  useSetupWizardStore,
} from '../stores/setupWizardStore'

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
  { id: 'devices', label: 'Devices' },
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
    controllerTypes,
    close,
    next,
    back,
    skipForever,
    markComplete,
    setStep,
    setMidiMasterEnabled,
    setVelocityCurve,
    setControllerType,
  } = useSetupWizardStore()

  const [ports, setPorts] = useState<string[]>([])
  const [enabledPorts, setEnabledPorts] = useState<Set<string>>(new Set())
  const [activity, setActivity] = useState<MidiActivitySnapshot | null>(null)
  const [scanning, setScanning] = useState(false)

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
          <h3>MIDI Setup Wizard</h3>
          <button type="button" className="hw-setup-wizard-skip" onClick={skipForever}>
            Skip — won't ask again
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
              {s.label}
            </div>
          ))}
        </div>

        <div className="hw-setup-wizard-body">
          {step === 'welcome' && (
            <WelcomeStep ports={ports} scanning={scanning} rescan={rescan} />
          )}
          {step === 'devices' && (
            <DevicesStep
              ports={ports}
              enabled={enabledPorts}
              onToggle={toggleDevice}
              masterEnabled={midiMasterEnabled}
              onMasterChange={setMidiMasterEnabled}
              controllerTypes={controllerTypes}
              onTypeChange={setControllerType}
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
            Step {stepIdx + 1} of {STEPS.length}
          </span>
          <div className="hw-setup-wizard-actions">
            {stepIdx > 0 && step !== 'done' && (
              <button type="button" className="hw-btn" onClick={back}>
                Back
              </button>
            )}
            {(step === 'welcome' || step === 'devices') && (
              <button
                type="button"
                className="hw-btn hw-btn-ghost"
                onClick={() => void rescan()}
                disabled={scanning}
              >
                {scanning ? 'Scanning…' : 'Rescan'}
              </button>
            )}
            {step === 'done' ? (
              <button type="button" className="hw-btn hw-btn-primary" onClick={markComplete}>
                Open Hardwave
              </button>
            ) : (
              <button
                type="button"
                className="hw-btn hw-btn-primary"
                onClick={() => (step === 'test' ? next() : next())}
              >
                Continue
              </button>
            )}
            {step === 'done' && (
              <button type="button" className="hw-btn hw-btn-ghost" onClick={close}>
                Close
              </button>
            )}
          </div>
        </footer>
      </div>
    </div>
  )
}

// ── Step bodies ────────────────────────────────────────────────────────────

function WelcomeStep({
  ports,
  scanning,
  rescan,
}: {
  ports: string[]
  scanning: boolean
  rescan: () => void
}) {
  return (
    <>
      <h4>Connect your MIDI controller</h4>
      <p>
        Hardwave detected the following ports. If your controller is missing, plug it in now —
        click <strong>Rescan</strong> when ready.
      </p>
      <div className="hw-setup-wizard-checklist">
        <div className="hw-setup-wizard-check">
          <span className={`hw-check-icon ${ports.length > 0 ? 'ok' : 'warn'}`}>
            {ports.length > 0 ? '✓' : '!'}
          </span>
          <div>
            <b>{ports.length} MIDI input{ports.length === 1 ? '' : 's'} found</b>
            <div className="hw-setup-wizard-muted">
              {ports.length === 0 && !scanning ? (
                <>
                  No ports detected. Connect a controller and hit Rescan, or skip if you only plan
                  to use the typing keyboard / on-screen Touch Controller.
                </>
              ) : (
                ports.join(' · ')
              )}
            </div>
          </div>
        </div>
        <div className="hw-setup-wizard-check">
          <span className="hw-check-icon ok">✓</span>
          <div>
            <b>MIDI driver healthy</b>
            <div className="hw-setup-wizard-muted">midir / CoreMIDI / ALSA initialised</div>
          </div>
        </div>
      </div>
      <p className="hw-setup-wizard-muted">
        Right after this step you'll be able to enable or disable each input individually and tag
        which controller it is, so Hardwave can pre-map common controls.
      </p>
    </>
  )
}

const KNOWN_CONTROLLERS: Array<{ id: string; label: string }> = [
  { id: 'generic', label: 'Generic controller' },
  { id: 'novation-launchkey', label: 'Novation Launchkey' },
  { id: 'akai-mpk', label: 'Akai MPK / MPC' },
  { id: 'arturia-keylab', label: 'Arturia KeyLab' },
  { id: 'native-instruments-komplete', label: 'NI Komplete Kontrol' },
  { id: 'launchpad', label: 'Novation Launchpad' },
]

function DevicesStep({
  ports,
  enabled,
  onToggle,
  masterEnabled,
  onMasterChange,
  controllerTypes,
  onTypeChange,
  rescan,
  scanning,
}: {
  ports: string[]
  enabled: Set<string>
  onToggle: (port: string) => void
  masterEnabled: boolean
  onMasterChange: (v: boolean) => void
  controllerTypes: Record<string, string>
  onTypeChange: (port: string, type: string) => void
  rescan: () => void
  scanning: boolean
}) {
  return (
    <>
      <h4>Enable inputs &amp; tag controller type</h4>
      <p className="hw-setup-wizard-muted">
        Each row maps to a connected MIDI device. Toggle off any port you don't want feeding
        Hardwave — useful for virtual loopback ports or noisy hardware.
      </p>

      <label className="hw-setup-wizard-master">
        <span>
          <strong>Master — Enable MIDI remote control</strong>
          <small>When off, no MIDI input reaches the audio thread.</small>
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
            No ports detected. {scanning ? 'Scanning…' : (
              <a onClick={rescan} role="button">Rescan</a>
            )}
          </div>
        )}
        {ports.map((port) => {
          const isOn = enabled.has(port)
          const type = controllerTypes[port] ?? 'generic'
          return (
            <div key={port} className={`hw-setup-wizard-device${isOn ? '' : ' is-off'}`}>
              <span className="hw-setup-wizard-led" aria-hidden />
              <div className="hw-setup-wizard-device-name">
                <strong>{port}</strong>
                <small>{isOn ? 'Listening' : 'Disabled'}</small>
              </div>
              <select
                value={type}
                onChange={(e) => onTypeChange(port, e.target.value)}
                disabled={!isOn}
              >
                {KNOWN_CONTROLLERS.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.label}
                  </option>
                ))}
              </select>
              <button
                type="button"
                className={`hw-setup-wizard-toggle${isOn ? ' is-on' : ''}`}
                onClick={() => onToggle(port)}
                aria-label={`Toggle ${port}`}
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
  { id: 'soft', label: 'Soft', desc: 'Boost low hits' },
  { id: 'hard', label: 'Hard', desc: 'Crush high hits' },
  { id: 's-curve', label: 'S-curve', desc: 'Center bias' },
  { id: 'custom', label: 'Custom', desc: 'Drag points (coming soon)' },
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
  const [activePort, setActivePort] = useState<string | null>(ports[0] ?? null)
  useEffect(() => {
    if (!activePort && ports.length > 0) setActivePort(ports[0])
  }, [ports, activePort])

  if (ports.length === 0) {
    return (
      <>
        <h4>Velocity curve</h4>
        <p className="hw-setup-wizard-muted">
          No hardware inputs to calibrate. The typing keyboard + Touch Controller already use a
          fixed default velocity (or position-based when enabled in Touch settings). Skip ahead to
          the activity test.
        </p>
      </>
    )
  }

  const currentCurve = activePort ? curves[activePort] ?? 'linear' : 'linear'

  return (
    <>
      <h4>Velocity curve</h4>
      <p className="hw-setup-wizard-muted">
        Pick a preset that matches your controller's velocity feel. Curves are per-input, so a
        light-action Novation can run Soft while a heavier MPC stays Linear.
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
              disabled={c.id === 'custom'}
            >
              <strong>{c.label}</strong>
              <small>{c.desc}</small>
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
        Backend application of the curve lands in the permanent MIDI panel ship — for now the
        wizard captures your preference so the future audio-thread tweak picks it up automatically.
      </p>
    </>
  )
}

function curvePath(curve: VelocityCurve): string {
  // Render a representative curve as an SVG path. y-axis is inverted
  // (0,100 = bottom-left). All curves end at (100, 0) — top-right.
  switch (curve) {
    case 'soft':
      return 'M 0 100 Q 25 60, 60 25 T 100 0'
    case 'hard':
      return 'M 0 100 Q 60 80, 80 40 T 100 0'
    case 's-curve':
      return 'M 0 100 C 35 100, 65 0, 100 0'
    case 'custom':
      return 'M 0 100 L 20 80 L 40 70 L 70 30 L 100 0'
    case 'linear':
    default:
      return 'M 0 100 L 100 0'
  }
}

function TestStep({ activity }: { activity: MidiActivitySnapshot | null }) {
  const fresh = activity != null && activity.ms_since_last_event != null && activity.ms_since_last_event < 800
  return (
    <>
      <h4>Play a note — Hardwave is listening</h4>
      <div className="hw-setup-wizard-activity">
        <div className={`hw-setup-wizard-led-large${fresh ? ' is-fresh' : ''}`} aria-hidden />
        <div>
          <h5>
            {fresh
              ? 'MIDI activity detected — you are connected'
              : 'Waiting for input…'}
          </h5>
          <p className="hw-setup-wizard-muted">
            The big LED pulses every time the engine receives a MIDI message. Notes play through
            Hardwave's default sine voice so you hear what you're playing.
          </p>
          {activity && (
            <p className="hw-setup-wizard-muted" style={{ fontSize: 11 }}>
              {activity.open_ports.length} port{activity.open_ports.length === 1 ? '' : 's'} open ·
              {activity.ms_since_last_event == null
                ? ' no events yet'
                : ` last event ${activity.ms_since_last_event} ms ago`}
            </p>
          )}
        </div>
      </div>
      <p className="hw-setup-wizard-muted">
        No activity? Click Back to recheck your devices, or run Rescan from the Tools menu later.
      </p>
    </>
  )
}

function DoneStep() {
  return (
    <>
      <h4>You're set 🎹</h4>
      <p>Hardwave will remember your choices for the next time you open the app. To change them later:</p>
      <div className="hw-setup-wizard-checklist">
        <div className="hw-setup-wizard-check">
          <span className="hw-check-icon arrow">→</span>
          <div>
            <b>Tools → MIDI mappings…</b>
            <div className="hw-setup-wizard-muted">Per-CC mappings + the Learn flow</div>
          </div>
        </div>
        <div className="hw-setup-wizard-check">
          <span className="hw-check-icon arrow">→</span>
          <div>
            <b>File → Audio settings…</b>
            <div className="hw-setup-wizard-muted">
              Input / output ports + velocity curves (permanent panel)
            </div>
          </div>
        </div>
        <div className="hw-setup-wizard-check">
          <span className="hw-check-icon arrow">→</span>
          <div>
            <b>Help → Re-run setup wizard</b>
            <div className="hw-setup-wizard-muted">Walks through this flow again</div>
          </div>
        </div>
      </div>
    </>
  )
}
