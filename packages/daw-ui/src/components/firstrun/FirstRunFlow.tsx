import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { t, tf, useLanguage } from '../../i18n'
import { midiInputOn, useSetupWizardStore } from '../../stores/setupWizardStore'
import { STARTERS } from '../TemplateDialog'
import { WelcomeScreen } from '../WelcomeScreen'
import { TestToneStrip, useTestTone } from '../settings/TestToneStrip'
import type { MidiInputSeen } from '../../lib/midiHotplug'
import {
  DEFAULT_SONG, FLOW_STEPS, finishLabel, nextStep, noteName, previousStep,
  stepCounter, stepStates, type FlowStep, type SongChoice,
} from './firstRunSteps'
import './FirstRunFlow.css'

/**
 * The first run, in one window: the start screen, the sound card with a
 * test tone, the controller (only when one is found), and a song to start
 * from. After it the producer is in a song that plays, and it does not come
 * back. Help > Re-run setup opens it again from the start.
 */

export interface FirstRunFlowProps {
  recentProjects: string[]
  onOpenRecent: (path: string) => void
  onOpenProject: () => void
  /** Open the empty project or a starter song. */
  onStartSong: (choice: Exclude<SongChoice, 'open'>) => void
}

/** The stepper on top, the same on every step. */
export function FlowStepper({ step, controllersFound }: { step: FlowStep; controllersFound: boolean }) {
  useLanguage()
  const states = stepStates(step, controllersFound)
  return (
    <div className="hw-fr-stepper" data-testid="first-run-stepper">
      {FLOW_STEPS.map((s, i) => (
        <span key={s.id} className={`is-${states[s.id]}`} data-state={states[s.id]}>
          {i + 1} {t(s.label)}
        </span>
      ))}
    </div>
  )
}

/** The controllers there now: the inputs that play, not desks or virtual
 *  ports. */
async function findControllers(): Promise<string[]> {
  try {
    const inputs = await invoke<MidiInputSeen[]>('list_midi_input_kinds')
    return inputs.filter(i => i.kind === 'keyboard').map(i => i.port)
  } catch {
    return []
  }
}

export function FirstRunFlow({ recentProjects, onOpenRecent, onOpenProject, onStartSong }: FirstRunFlowProps) {
  useLanguage()
  const visible = useSetupWizardStore(s => s.visible)
  const step = useSetupWizardStore(s => s.step)
  const setStep = useSetupWizardStore(s => s.setStep)
  const [controllers, setControllers] = useState<string[]>([])
  const [song, setSong] = useState<SongChoice>(DEFAULT_SONG)

  // Look for controllers when the window opens, so the stepper can say
  // whether there is a Controller step.
  useEffect(() => {
    if (!visible) return
    setSong(DEFAULT_SONG)
    void findControllers().then(setControllers)
  }, [visible])

  const found = controllers.length > 0

  const skip = useCallback(() => {
    // Sound stays where it is (the system's default unless one was picked)
    // and the song is the empty project.
    useSetupWizardStore.getState().skipForever()
    onStartSong('empty')
  }, [onStartSong])

  if (!visible) return null

  if (step === 'start') {
    return (
      <WelcomeScreen
        recentProjects={recentProjects}
        onOpenRecent={(path) => { useSetupWizardStore.getState().skipForever(); onOpenRecent(path) }}
        onOpenProject={() => { useSetupWizardStore.getState().skipForever(); onOpenProject() }}
        firstRun={{
          stepper: <FlowStepper step="start" controllersFound={found} />,
          onSetUp: () => setStep('sound'),
          onSkip: skip,
        }}
      />
    )
  }

  /** The end of setup: the window closes first, so a Save changes prompt
   *  (Help > Re-run setup with a song open) is not hidden behind it. */
  const finish = (choice: SongChoice) => {
    useSetupWizardStore.getState().markComplete()
    if (choice === 'open') onOpenProject()
    else onStartSong(choice)
  }
  const goNext = async () => {
    if (step === 'song') {
      finish(song)
      return
    }
    // A controller plugged in during the Sound step still gets its step.
    const now = step === 'sound' ? await findControllers() : controllers
    if (step === 'sound') setControllers(now)
    const to = nextStep(step, now.length > 0)
    if (to) setStep(to)
  }
  const goBack = () => {
    const to = previousStep(step, found)
    if (to) setStep(to)
  }

  return (
    <div className="hw-fr-backdrop" role="dialog" aria-modal="true" aria-label={t('Set up Hardwave')}>
      <div className="hw-fr" data-testid="first-run" data-step={step}>
        <header className="hw-fr-head">
          <h3>{t('Set up Hardwave')}</h3>
          <button type="button" className="hw-fr-skip" onClick={skip}>{t('Skip setup')}</button>
        </header>
        <div className="hw-fr-stepwrap">
          <FlowStepper step={step} controllersFound={found} />
        </div>
        <div className="hw-fr-body">
          {step === 'sound' && <SoundStep />}
          {step === 'controller' && <ControllerStep ports={controllers} />}
          {step === 'song' && <SongStep picked={song} onPick={setSong} onOpen={finish} />}
        </div>
        <footer className="hw-fr-foot">
          <span className="hw-fr-meta">{stepCounter(step)}</span>
          <button type="button" className="hw-fr-btn" onClick={goBack}>{t('Back')}</button>
          <button type="button" className="hw-fr-btn is-primary" onClick={() => { void goNext() }}>
            {step === 'song' ? finishLabel(song) : t('Continue')}
          </button>
        </footer>
      </div>
    </div>
  )
}

// ── Sound ──────────────────────────────────────────────────────────────────

interface OutputDevice {
  name: string
  is_default: boolean
  sample_rates: number[]
}

const BUFFER_CHOICES = [128, 256, 512, 1024]

interface AudioChoice {
  device: string | null
  sampleRate: number
  bufferSize: number
}

/** Where the sound comes out. Picking a device applies it and plays the
 *  test tone on it, so the producer hears which device is live. Continue
 *  always works, so nobody gets stuck here. */
function SoundStep() {
  useLanguage()
  const [devices, setDevices] = useState<OutputDevice[]>([])
  const [device, setDevice] = useState<string | null>(null)
  const [sampleRate, setSampleRate] = useState(48000)
  const [bufferSize, setBufferSize] = useState(512)
  const tone = useTestTone()
  const { play, failed } = tone
  const applying = useRef(false)
  // A pick made while the last one is still being applied: applied next,
  // so the device on screen is the one that plays.
  const waiting = useRef<AudioChoice | null>(null)

  const apply = useCallback(async (choice: AudioChoice) => {
    if (applying.current) {
      waiting.current = choice
      return
    }
    applying.current = true
    let next: AudioChoice | null = choice
    while (next) {
      waiting.current = null
      try {
        await invoke('set_audio_config', { ...next })
        await play()
      } catch (e) {
        failed(String(e))
      }
      next = waiting.current
    }
    applying.current = false
  }, [play, failed])

  // Read what plays now, and play the tone on it: the strip names the
  // device that is live before anything is picked.
  useEffect(() => {
    let cancelled = false
    Promise.all([
      invoke<OutputDevice[]>('get_audio_devices'),
      invoke<{ device: string | null; sample_rate: number; buffer_size: number }>('get_audio_config'),
    ]).then(([devs, cfg]) => {
      if (cancelled) return
      setDevices(devs)
      setDevice(cfg.device ?? devs.find(d => d.is_default)?.name ?? null)
      setSampleRate(cfg.sample_rate)
      setBufferSize(cfg.buffer_size)
      void play()
    }).catch(e => { if (!cancelled) failed(String(e)) })
    return () => { cancelled = true }
  }, [play, failed])

  const selected = devices.find(d => d.name === device)
  const rates = selected?.sample_rates?.length ? selected.sample_rates : [44100, 48000]
  const latencyMs = ((bufferSize / sampleRate) * 1000).toFixed(1)
  const pick = (next: Partial<AudioChoice>) => {
    const choice = { device, sampleRate, bufferSize, ...next }
    // A rate the new device does not offer moves to one it does.
    const dev = devices.find(d => d.name === choice.device)
    if (dev?.sample_rates?.length && !dev.sample_rates.includes(choice.sampleRate)) {
      choice.sampleRate = dev.sample_rates.includes(48000) ? 48000 : dev.sample_rates[0]
    }
    setDevice(choice.device)
    setSampleRate(choice.sampleRate)
    setBufferSize(choice.bufferSize)
    void apply(choice)
  }

  return (
    <>
      <h4>{t('Where should Hardwave play?')}</h4>
      <p>{t('Pick your audio interface if you have one, not the laptop speakers. A short tone plays on what you pick, so you hear which device is live.')}</p>
      <div className="hw-fr-audio">
        <label>
          {t('Output device')}
          <select value={device ?? ''} onChange={e => pick({ device: e.target.value || null })}>
            {devices.map(d => <option key={d.name} value={d.name}>{d.name}</option>)}
          </select>
        </label>
        <label>
          {t('Sample rate')}
          <select value={sampleRate} onChange={e => pick({ sampleRate: Number(e.target.value) })}>
            {rates.map(r => <option key={r} value={r}>{r} Hz</option>)}
          </select>
        </label>
        <label>
          {t('Buffer size')}
          <select value={bufferSize} onChange={e => pick({ bufferSize: Number(e.target.value) })}>
            {BUFFER_CHOICES.map(b => <option key={b} value={b}>{tf('{n} samples', { n: b })}</option>)}
          </select>
        </label>
      </div>
      <TestToneStrip
        state={tone.state}
        meters={tone.meters}
        detail={(on) => tf('{rate} kHz, {buffer} samples', { rate: on.sampleRate / 1000, buffer: on.bufferSize })}
        onPlayAgain={() => { void play() }}
        onRetry={() => { void apply({ device, sampleRate, bufferSize }) }}
      />
      <p className="hw-fr-muted" style={{ marginTop: 14 }}>
        <b>{tf('Latency about {ms} ms.', { ms: latencyMs })}</b>{' '}
        {t('512 samples is a safe start; crackles mean the buffer is too small. You can change all of this later under File > Audio settings.')}
      </p>
    </>
  )
}

// ── Controller ─────────────────────────────────────────────────────────────

interface Activity {
  ms_since_last_event: number | null
  last_note: { note: number; velocity: number } | null
}

/** The controllers found, on, with a live readout of the last key played.
 *  The keys sound through a plain sine voice while this step shows: a new
 *  song has no instrument to play them on. */
function ControllerStep({ ports }: { ports: string[] }) {
  useLanguage()
  const midiInputs = useSetupWizardStore(s => s.midiInputs)
  const [activity, setActivity] = useState<Activity | null>(null)

  // Each one found is on, unless it was turned off before.
  useEffect(() => {
    const store = useSetupWizardStore.getState()
    for (const port of ports) {
      if (midiInputOn(store.midiInputs, port)) {
        store.setMidiInputEnabled(port, true).catch(err => console.warn('open_midi_input failed:', err))
      }
    }
  }, [ports])

  useEffect(() => {
    invoke('set_check_voice', { enabled: true }).catch(() => {})
    const id = window.setInterval(() => {
      invoke<Activity>('get_midi_activity').then(setActivity).catch(() => { /* engine busy */ })
    }, 150)
    return () => {
      window.clearInterval(id)
      invoke('set_check_voice', { enabled: false }).catch(() => {})
    }
  }, [])

  const toggle = (port: string) => {
    const on = midiInputOn(useSetupWizardStore.getState().midiInputs, port)
    useSetupWizardStore.getState().setMidiInputEnabled(port, !on)
      .catch(err => console.warn('MIDI input toggle failed:', err))
  }

  const fresh = activity?.ms_since_last_event != null && activity.ms_since_last_event < 800
  const last = activity?.last_note ?? null

  return (
    <>
      <h4>{ports.length === 1 ? t('Controller found') : t('Controllers found')}</h4>
      <p>
        {ports.length === 1
          ? t('Hardwave found a MIDI controller and turned it on. Play a few keys to check it.')
          : t('Hardwave found these MIDI controllers and turned them on. Play a few keys to check them.')}
      </p>
      {ports.map(port => {
        const on = midiInputOn(midiInputs, port)
        return (
          <div key={port} className={`hw-fr-device${on ? '' : ' is-off'}`}>
            <span className="hw-fr-led" aria-hidden />
            <div style={{ flex: 1, minWidth: 0 }}>
              <b>{port}</b>
              <small>{on ? t('On, listening') : t('Off')}</small>
            </div>
            <button
              type="button"
              className={`hw-fr-toggle${on ? ' is-on' : ''}`}
              aria-pressed={on}
              aria-label={tf('Turn {name} on or off', { name: port })}
              onClick={() => toggle(port)}
            />
          </div>
        )
      })}
      <div className="hw-fr-activity" data-testid="first-run-activity">
        <span className={`hw-fr-bigled${fresh ? ' is-lit' : ''}`} aria-hidden />
        <div style={{ flex: 1, minWidth: 0 }}>
          <b>
            {last
              ? tf('{note} received, velocity {velocity}', { note: noteName(last.note), velocity: last.velocity })
              : t('Play a key')}
          </b>
          <div className="hw-fr-muted" style={{ fontSize: 11.5 }}>
            {t('You hear the notes through a plain built-in synth voice.')}
          </div>
        </div>
        <MiniKeys note={last?.note ?? null} />
      </div>
      <p className="hw-fr-muted" style={{ margin: 0 }}>
        {t('Plug in another controller later and it turns on by itself, with a message you can undo. How hard the keys have to be hit (the velocity curve) is in Settings > MIDI.')}
      </p>
    </>
  )
}

/** Two octaves of keys, from the C below the last note, with it lit. */
const WHITE_OF = [0, -1, 1, -1, 2, 3, -1, 4, -1, 5, -1, 6]
const BLACK_LEFT: Record<number, number> = { 1: 13, 3: 32, 6: 70, 8: 89, 10: 108 }
function MiniKeys({ note }: { note: number | null }) {
  const base = note == null ? 48 : note - (note % 12)
  const hit = note == null ? -1 : note - base
  const whites = Array.from({ length: 15 }, (_, i) => i)
  const whiteHit = hit >= 0 && WHITE_OF[hit % 12] >= 0 ? Math.floor(hit / 12) * 7 + WHITE_OF[hit % 12] : -1
  return (
    <div className="hw-fr-keys" style={{ width: 285 }} aria-hidden>
      {whites.map(i => <span key={i} className={`w${i === whiteHit ? ' is-hit' : ''}`} />)}
      {[0, 1].flatMap(octave => Object.entries(BLACK_LEFT).map(([semi, left]) => {
        const at = octave * 12 + Number(semi)
        return <span key={at} className={`k${at === hit ? ' is-hit' : ''}`} style={{ left: left + octave * 133 }} />
      }))}
    </div>
  )
}

// ── Song ───────────────────────────────────────────────────────────────────

function SongStep({ picked, onPick, onOpen }: {
  picked: SongChoice
  onPick: (choice: SongChoice) => void
  onOpen: (choice: SongChoice) => void
}) {
  useLanguage()
  const card = (choice: SongChoice, title: string, detail: string, bpm?: number) => (
    <button
      key={choice}
      type="button"
      className={`hw-fr-card${picked === choice ? ' is-picked' : ''}`}
      aria-pressed={picked === choice}
      onClick={() => onPick(choice)}
      // A double click opens it at once.
      onDoubleClick={() => onOpen(choice)}
    >
      <b>{title}</b>{bpm ? <em>{bpm} BPM</em> : null}
      <p>{detail}</p>
    </button>
  )
  return (
    <>
      <h4>{t('Start with a song')}</h4>
      <p>{t('Each one plays as soon as you press Play. Everything in it is there to be replaced.')}</p>
      <div className="hw-fr-cards">
        {STARTERS.map(s => card(`starter:${s.id}`, t(s.title), t(s.detail), s.bpm))}
      </div>
      <div className="hw-fr-cards">
        {card('empty', t('Empty project'), t('Kick, Clap, Hat and Snare loaded, no notes yet.'))}
        {card('open', t('Open project…'), t('A song you already have, or an FL Studio project.'))}
      </div>
    </>
  )
}
