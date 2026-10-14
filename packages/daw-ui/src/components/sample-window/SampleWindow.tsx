/**
 * The sample window: a sample's channel settings, as FL has them. Approved
 * as version D of the mockup on 13 October 2026 (suite:
 * daw-sample-window-mockup): FL's layout and grid, the DAW's own colours,
 * flat.
 *
 * It opens on two things:
 *   - an audio clip, by double-clicking it. FL keeps these settings per
 *     channel, so they belong to the sample here: a change reaches every
 *     clip that plays the same file, wherever it sits.
 *   - a sampler channel, from the channel rack. Its settings are the
 *     channel's sampler (crates/hardwave-native-plugins/src/sampler.rs):
 *     the sample's offset and loop on the first page, the volume
 *     envelope, mono, cut itself, porta and the root note on the second.
 *
 * What the DAW cannot do yet is shown switched off, as FL greys what does
 * not apply, and says so when pointed at.
 */
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { invokeOrToast } from '../../api/invoke'
import { useSampleWindowStore } from '../../stores/sampleWindowStore'
import { useTrackStore, type ClipInfo, type TrackInfo } from '../../stores/trackStore'
import { useTransportStore } from '../../stores/transportStore'
import { useHistoryStore } from '../../stores/historyStore'
import { useProjectStore } from '../../stores/projectStore'
import { PPQ } from '../../utils/meter'
import { FlatBox, FlatCheck, FlatKnob, FlatPanel, FlatSelect, FlatSwitch } from '../ui/flat/Flat'
import type { PluginParam } from '../plugins/pluginDraw'
import './sampleWindow.css'

// ------------------------------------------------------------------ shared

const NOT_YET = 'Not in this version yet'
const FOR_SAMPLER = 'For sampler channels; comes with them'
const FOR_CLIPS = 'For audio clips; a sampler channel does not stretch yet'
const ms = (v: number) => (v <= 0 ? '0 ms' : v < 1000 ? `${v < 10 ? v.toFixed(1) : Math.round(v)} ms` : `${(v / 1000).toFixed(2)} s`)
const db = (v: number) => (v <= -60 ? '-inf dB' : `${v > 0 ? '+' : ''}${v.toFixed(1)} dB`)
const pct = (v: number) => `${Math.round(v * 100)} %`
const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
/** C4 is 60, as the piano roll names its keys. */
const noteName = (n: number) => `${NOTE_NAMES[n % 12]}${Math.floor(n / 12) - 1}`
const NOTE_OPTIONS = Array.from({ length: 128 }, (_, i) => noteName(i))
const baseName = (path: string) => path.split(/[\\/]/).pop() || path

type Page = 'sample' | 'envelope'

export function SampleWindow() {
  const target = useSampleWindowStore((s) => s.target)
  if (!target) return null
  if (target.kind === 'clip') {
    return <ClipWindow key={`clip/${target.trackId}/${target.clipId}`} trackId={target.trackId} clipId={target.clipId} />
  }
  return <SamplerWindow key={`sampler/${target.trackId}/${target.slotId}`} trackId={target.trackId} slotId={target.slotId} />
}

/** The window itself: its title bar to drag it by, its pages' tabs, the
 *  controls at the top right, and whatever page is open. */
function Frame({ name, color, hasEnvelope, page, onPage, top, children }: {
  name: string
  color: string
  /** Whether the second page applies: it does for a sampler channel. */
  hasEnvelope: boolean
  page: Page
  onPage: (p: Page) => void
  top: ReactNode
  children: ReactNode
}) {
  const close = useSampleWindowStore((s) => s.close)
  // The window sits where it was dragged; it opens near the top, centred.
  const [pos, setPos] = useState(() => ({ x: Math.max(16, (window.innerWidth - 720) / 2), y: 96 }))
  useEffect(() => {
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') close() }
    window.addEventListener('keydown', esc)
    return () => window.removeEventListener('keydown', esc)
  }, [close])

  return (
    <div className="hwf sw-win" style={{ left: pos.x, top: pos.y }} role="dialog" aria-label={`${name} settings`}>
      <div
        className="sw-title"
        onPointerDown={(e) => {
          if ((e.target as HTMLElement).closest('button')) return
          const x0 = e.clientX - pos.x
          const y0 = e.clientY - pos.y
          const move = (ev: PointerEvent) => setPos({
            x: Math.max(0, Math.min(window.innerWidth - 120, ev.clientX - x0)),
            y: Math.max(28, Math.min(window.innerHeight - 40, ev.clientY - y0)),
          })
          const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up) }
          window.addEventListener('pointermove', move)
          window.addEventListener('pointerup', up)
        }}
      >
        <span className="sw-swatch" style={{ background: color }} />
        <span className="sw-name">{name}</span>
        <span className="sw-grow" />
        <button type="button" className="sw-x" title="Close (Esc)" onClick={close}>✕</button>
      </div>

      <div className="sw-top">
        <div
          className={`sw-tab is-pick${page === 'sample' ? ' is-on' : ''}`} title="Sample" role="tab" aria-selected={page === 'sample'}
          onClick={() => onPage('sample')}
        >
          <svg width="22" height="16" viewBox="0 0 30 22" fill="none" strokeWidth="2.2" strokeLinecap="round"><path d="M2 11h4l2-5 3 10 3-14 3 18 3-12 2 6 2-3h4" /></svg>
        </div>
        <div
          className={`sw-tab${hasEnvelope ? ' is-pick' : ' is-off'}${page === 'envelope' ? ' is-on' : ''}`}
          title={hasEnvelope ? 'Envelope' : `Envelope. ${FOR_SAMPLER}`} role="tab" aria-selected={page === 'envelope'}
          onClick={() => { if (hasEnvelope) onPage('envelope') }}
        >
          <svg width="20" height="18" viewBox="0 0 26 24" fill="none" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="4" cy="20" r="2.5" /><circle cx="22" cy="20" r="2.5" /><path d="M5.5 18 12 3l3 7 5.5 8" /></svg>
        </div>
        <div className="sw-tab is-off" title={`More. ${NOT_YET}`}><svg width="18" height="18" viewBox="0 0 24 24" fill="none" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M14.5 6.5a4.5 4.5 0 0 0-6 5.5L3 17.5 6.5 21 12 15.5a4.5 4.5 0 0 0 5.5-6l-3 3-2.5-.5-.5-2.5z" /></svg></div>
        <span className="sw-grow" />
        {top}
      </div>

      {children}
    </div>
  )
}

/** The track's number and where it plays into, for the Track box. */
function TrackBox({ track, tracks }: { track: TrackInfo; tracks: TrackInfo[] }) {
  const output = track.outputBus ? tracks.find((t) => t.id === track.outputBus) : null
  const trackNumber = tracks.filter((t) => t.kind !== 'Master').findIndex((t) => t.id === track.id) + 1
  return <FlatBox label="Track" value={trackNumber || '-'} small={output?.name ?? 'Master'} hint={`${track.name} plays into ${output?.name ?? 'the master'}; route it in the mixer`} />
}

/** What FL has on the first page that the DAW cannot do yet. */
function ContentPanel() {
  return (
    <FlatPanel title="Content">
      <div className="hwf-checks">
        <FlatCheck label="Keep on disk" on={false} disabled hint={NOT_YET} />
        <FlatCheck label="Load regions" on={false} disabled hint={NOT_YET} />
        <FlatCheck label="Resample" on={false} disabled hint={NOT_YET} />
        <FlatCheck label="Load slice markers" on={false} disabled hint={NOT_YET} />
      </div>
      <div className="sw-row">
        <span className="sw-lead">Declicking mode</span>
        <FlatSelect value="Out only" options={['Out only'] as const} disabled style={{ flex: 1 }} />
      </div>
    </FlatPanel>
  )
}

// ------------------------------------------------------------------ an audio clip

/** What the window shows and changes, in the units it shows them in. */
interface Values {
  muted: boolean
  gainDb: number
  pan: number
  pitch: number
  stretch: number
  resample: boolean
  reversed: boolean
  polarityInverted: boolean
  swapStereo: boolean
  normalize: boolean
  removeDc: boolean
  fadeInMs: number
  fadeOutMs: number
}

/** The command's own names for the same things. */
type Settings = Partial<{
  muted: boolean
  gainDb: number
  pan: number
  pitchSemitones: number
  stretchRatio: number
  resample: boolean
  reversed: boolean
  polarityInverted: boolean
  swapStereo: boolean
  normalize: boolean
  removeDc: boolean
  fadeInMs: number
  fadeOutMs: number
}>

function valuesOf(clip: ClipInfo): Values {
  const t = clip.treatment
  return {
    muted: clip.muted,
    gainDb: clip.gainDb,
    pan: t?.pan ?? 0,
    pitch: clip.pitchSemitones,
    stretch: clip.stretchRatio || 1,
    resample: t?.resample ?? false,
    reversed: clip.reversed,
    polarityInverted: t?.polarityInverted ?? false,
    swapStereo: t?.swapStereo ?? false,
    normalize: t?.normalize ?? false,
    removeDc: t?.removeDc ?? false,
    fadeInMs: clip.fadeInMs ?? 0,
    fadeOutMs: clip.fadeOutMs ?? 0,
  }
}

function settingsOf(v: Partial<Values>): Settings {
  const s: Settings = {}
  if (v.muted !== undefined) s.muted = v.muted
  if (v.gainDb !== undefined) s.gainDb = v.gainDb
  if (v.pan !== undefined) s.pan = v.pan
  if (v.pitch !== undefined) s.pitchSemitones = v.pitch
  if (v.stretch !== undefined) s.stretchRatio = v.stretch
  if (v.resample !== undefined) s.resample = v.resample
  if (v.reversed !== undefined) s.reversed = v.reversed
  if (v.polarityInverted !== undefined) s.polarityInverted = v.polarityInverted
  if (v.swapStereo !== undefined) s.swapStereo = v.swapStereo
  if (v.normalize !== undefined) s.normalize = v.normalize
  if (v.removeDc !== undefined) s.removeDc = v.removeDc
  if (v.fadeInMs !== undefined) s.fadeInMs = v.fadeInMs
  if (v.fadeOutMs !== undefined) s.fadeOutMs = v.fadeOutMs
  return s
}

const MODES = ['Resample', 'Stretch'] as const
const MODE_HINT: Record<(typeof MODES)[number], string> = {
  Resample: 'speed and pitch together, like a tape',
  Stretch: 'length and pitch apart',
}
/** How much longer than its file the sample lasts: the stretch, and in
 *  resample also the pitch (AudioClip::time_factor). */
const timeFactor = (v: Values) => v.stretch / (v.resample ? Math.pow(2, v.pitch / 12) : 1)

function ClipWindow({ trackId, clipId }: { trackId: string; clipId: string }) {
  const tracks = useTrackStore((s) => s.tracks)
  const bpm = useTransportStore((s) => s.bpm)
  const colorOverride = useTransportStore((s) => s.clipColorOverrides[clipId])
  const track = tracks.find((t) => t.id === trackId)
  const clip = track?.clips.find((c) => c.id === clipId)

  const [v, setV] = useState<Values | null>(() => (clip ? valuesOf(clip) : null))
  const [range, setRange] = useState(() => Math.max(2, Math.ceil(Math.abs(clip?.pitchSemitones ?? 0))))
  const [page, setPage] = useState<Page>('sample')
  const gesture = useRef(false)
  // What arrives from the store wins, except in the middle of a gesture.
  useEffect(() => {
    if (clip && !gesture.current) setV(valuesOf(clip))
  }, [clip])

  // A change: shown at once, sent to the engine. A gesture's first move
  // takes the undo snapshot; its end reloads the song and names the step.
  const sendTimer = useRef<number | null>(null)
  const pending = useRef<Settings>({})
  const flush = (snapshot: boolean) => {
    const settings = pending.current
    pending.current = {}
    if (sendTimer.current !== null) { window.clearTimeout(sendTimer.current); sendTimer.current = null }
    if (Object.keys(settings).length === 0) return Promise.resolve()
    return invokeOrToast('set_sample_settings', { trackId, clipId, settings, snapshot }, { message: 'Changing the sample failed', retry: false })
  }
  const change = (patch: Partial<Values>, done: boolean, label: string) => {
    // A click on a knob that did not turn it is not a change.
    const same = v !== null && Object.entries(patch).every(([k, val]) => v[k as keyof Values] === val)
    if (done && !gesture.current && same) return
    setV((prev) => (prev ? { ...prev, ...patch } : prev))
    pending.current = { ...pending.current, ...settingsOf(patch) }
    const first = !gesture.current
    if (!done) {
      gesture.current = true
      if (first) { void flush(true); return }
      if (sendTimer.current === null) sendTimer.current = window.setTimeout(() => { void flush(false) }, 40)
      return
    }
    gesture.current = false
    void flush(first).then(async () => {
      useProjectStore.getState().markDirty()
      useHistoryStore.getState().push(label)
      await useTrackStore.getState().fetchTracks()
    })
  }

  const [peaks, setPeaks] = useState<number[][]>([])
  useEffect(() => {
    if (!clip) return
    let live = true
    invoke<number[][]>('get_waveform_peaks', { sourceId: clip.source_id, numBuckets: 700 })
      .then((p) => { if (live) setPeaks(p) })
      .catch(() => { if (live) setPeaks([]) })
    return () => { live = false }
  }, [clip?.source_id]) // eslint-disable-line react-hooks/exhaustive-deps

  if (!track || !clip || !v) return null

  const color = colorOverride || track.color || '#10b981'
  const seconds = clip.sourceFrames && clip.sourceRate ? clip.sourceFrames / clip.sourceRate : 0
  const beatSec = 60 / Math.max(1, bpm)
  const naturalBeats = seconds / beatSec
  const beatsNow = naturalBeats * timeFactor(v)
  const fileName = baseName(clip.sourceFile || clip.source_id) || clip.name
  const rate = clip.sourceRate || 48_000
  const regionStart = (clip.sourceStart ?? 0) / rate
  const playsFor = (clip.length_ticks / PPQ) * beatSec / Math.max(0.01, timeFactor(v))
  const region = { start: regionStart, end: Math.min(seconds || regionStart + playsFor, regionStart + playsFor) }

  return (
    <Frame
      name={clip.name || fileName} color={color} hasEnvelope={false} page={page} onPage={setPage}
      top={(
        <>
          <FlatSwitch label="On" on={!v.muted} onToggle={() => change({ muted: !v.muted }, true, v.muted ? 'Sample on' : 'Sample off')} />
          <FlatKnob
            label="Pan" size={32} value={(v.pan + 1) / 2} def={0.5}
            text={`Pan ${v.pan === 0 ? 'centred' : `${Math.round(Math.abs(v.pan) * 100)}% ${v.pan < 0 ? 'left' : 'right'}`}`}
            onChange={(t, done) => change({ pan: Math.round((t * 2 - 1) * 100) / 100 }, done, 'Pan sample')}
            onReset={() => change({ pan: 0 }, true, 'Pan sample')}
          />
          <FlatKnob
            label="Vol" size={32} value={(v.gainDb + 60) / 72} def={60 / 72}
            text={`Volume ${db(v.gainDb)}`}
            onChange={(t, done) => change({ gainDb: Math.round((t * 72 - 60) * 10) / 10 }, done, 'Sample volume')}
            onReset={() => change({ gainDb: 0 }, true, 'Sample volume')}
          />
          <FlatKnob
            label="Pitch" size={32} value={(v.pitch + range) / (2 * range)} def={0.5}
            text={`Pitch ${v.pitch > 0 ? '+' : ''}${v.pitch.toFixed(2)} st`}
            onChange={(t, done) => change({ pitch: Math.round((t * 2 - 1) * range * 100) / 100 }, done, 'Pitch sample')}
            onReset={() => change({ pitch: 0 }, true, 'Pitch sample')}
          />
          <FlatBox label="Range" value={range} onStep={(by) => setRange((r) => Math.max(1, Math.min(48, r + by)))} hint={`The pitch knob turns ${range} semitones each way. Scroll to change.`} />
          <TrackBox track={track} tracks={tracks} />
        </>
      )}
    >
      <div className="sw-cols">
        <div className="sw-col">
          <FlatPanel>
            <div className="sw-file">
              <span className="hwf-lab" style={{ textTransform: 'none', fontSize: 12 }}>File</span>
              <span className="sw-file-name" title={clip.sourceFile || clip.source_id}>{fileName}</span>
            </div>
          </FlatPanel>
          <ContentPanel />
          <FlatPanel title="Playback" grow>
            <div className="hwf-krow">
              <FlatKnob label="Offset" value={0} def={0} text="" disabled offText={FOR_SAMPLER} />
              <FlatKnob label="Loop start" value={0} def={0} text="" disabled offText={FOR_SAMPLER} />
              <FlatKnob label="Loop end" value={1} def={1} text="" disabled offText={FOR_SAMPLER} />
            </div>
            <div className="hwf-checks">
              <FlatCheck label="Use loop points" on={false} disabled hint={FOR_SAMPLER} />
              <FlatCheck label="Ping pong loop" on={false} disabled hint={FOR_SAMPLER} />
            </div>
          </FlatPanel>
        </div>

        <div className="sw-col">
          <FlatPanel title="Time stretching">
            <div className="sw-row" style={{ alignItems: 'flex-start' }}>
              <div className="hwf-krow" style={{ flex: '0 0 192px' }}>
                <FlatKnob
                  label="Pitch" value={(v.pitch * 100 + 1200) / 2400} def={0.5}
                  text={`Pitch ${v.pitch > 0 ? '+' : ''}${Math.round(v.pitch * 100)} cents (the pitch at the top)`}
                  onChange={(t, done) => change({ pitch: Math.round(t * 2400 - 1200) / 100 }, done, 'Pitch sample')}
                  onReset={() => change({ pitch: 0 }, true, 'Pitch sample')}
                />
                <FlatKnob
                  label="Mul" value={(Math.log2(v.stretch) + 2) / 4} def={0.5}
                  text={`Length × ${v.stretch.toFixed(2)}`}
                  onChange={(t, done) => change({ stretch: Math.round(Math.pow(2, t * 4 - 2) * 100) / 100 }, done, 'Stretch sample')}
                  onReset={() => change({ stretch: 1 }, true, 'Stretch sample')}
                />
                <FlatKnob
                  label="Time" value={Math.min(1, beatsNow / 64)} def={Math.min(1, naturalBeats / 64)}
                  disabled={!seconds} offText="The file is not loaded"
                  text={`Lasts ${beatsNow.toFixed(beatsNow % 1 ? 2 : 0)} beats`}
                  onChange={(t, done) => {
                    // Whole quarter beats, as FL's TIME knob counts.
                    const beats = Math.max(0.25, Math.round(t * 64 * 4) / 4)
                    const pitchSpeed = v.resample ? Math.pow(2, v.pitch / 12) : 1
                    change({ stretch: Math.max(0.25, Math.min(4, (beats / naturalBeats) * pitchSpeed)) }, done, 'Fit sample to time')
                  }}
                  onReset={() => change({ stretch: 1 }, true, 'Fit sample to time')}
                />
              </div>
              <div className="sw-stack">
                <span className="hwf-lab">Mode</span>
                <FlatSelect
                  value={v.resample ? 'Resample' : 'Stretch'} options={MODES}
                  onPick={(m) => change({ resample: m === 'Resample' }, true, `Stretch mode ${m}`)}
                />
                <span className="sw-hint">{MODE_HINT[v.resample ? 'Resample' : 'Stretch']}</span>
              </div>
            </div>
          </FlatPanel>
          <FlatPanel title="Precomputed effects" grow>
            <div className="hwf-checks">
              <FlatCheck label="Remove DC offset" on={v.removeDc} onToggle={() => change({ removeDc: !v.removeDc }, true, 'Remove DC offset')} />
              <FlatCheck label="Reverse polarity" on={v.polarityInverted} onToggle={() => change({ polarityInverted: !v.polarityInverted }, true, 'Reverse polarity')} />
              <FlatCheck label="Normalize" on={v.normalize} onToggle={() => change({ normalize: !v.normalize }, true, 'Normalize sample')} />
              <FlatCheck label="Fade stereo" on={false} disabled hint={NOT_YET} />
              <FlatCheck label="Reverse" on={v.reversed} onToggle={() => change({ reversed: !v.reversed }, true, 'Reverse sample')} />
              <FlatCheck label="Swap stereo" on={v.swapStereo} onToggle={() => change({ swapStereo: !v.swapStereo }, true, 'Swap stereo')} />
            </div>
            <div className="hwf-krow">
              <FlatKnob label="Start" value={0} def={0} text="" disabled />
              <FlatKnob label="Length" value={1} def={1} text="" disabled />
              <FlatKnob
                label="In" value={Math.sqrt(v.fadeInMs / 2000)} def={0}
                text={`Fade in ${ms(v.fadeInMs)}`}
                onChange={(t, done) => change({ fadeInMs: Math.round(t * t * 2000) }, done, 'Sample fade in')}
                onReset={() => change({ fadeInMs: 0 }, true, 'Sample fade in')}
              />
              <FlatKnob
                label="Out" value={Math.sqrt(v.fadeOutMs / 2000)} def={0}
                text={`Fade out ${ms(v.fadeOutMs)}`}
                onChange={(t, done) => change({ fadeOutMs: Math.round(t * t * 2000) }, done, 'Sample fade out')}
                onReset={() => change({ fadeOutMs: 0 }, true, 'Sample fade out')}
              />
              <FlatKnob label="X-fade" value={0} def={0} text="" disabled offText={FOR_SAMPLER} />
              <FlatKnob label="Trim" value={0} def={0} text="" disabled />
            </div>
          </FlatPanel>
        </div>
      </div>

      <SampleView
        peaks={peaks} seconds={seconds} bpm={bpm} color={color} region={region} loop={null}
        fadeIn={v.fadeInMs / 1000 / Math.max(0.01, timeFactor(v))} fadeOut={v.fadeOutMs / 1000 / Math.max(0.01, timeFactor(v))}
        reversed={v.reversed}
      />
    </Frame>
  )
}

// ------------------------------------------------------------------ a sampler channel

/** The sampler's parameters, by id (crates/hardwave-native-plugins/src/sampler.rs). */
export const SP = {
  gain: 0, attack: 1, release: 2, delay: 3, hold: 4, decay: 5, sustain: 6, envelope: 7,
  offset: 8, loopStart: 9, loopEnd: 10, loop: 11, pingPong: 12, mono: 13, cutItself: 14, porta: 15,
  root: 16, pitch: 17,
} as const
/** The time knobs run on a square law: seconds = knob² × the longest they go. */
const TIME_MAX_S: Partial<Record<number, number>> = { [SP.attack]: 5, [SP.delay]: 5, [SP.hold]: 5, [SP.decay]: 10, [SP.release]: 10 }
const TIME_FLOOR_S: Partial<Record<number, number>> = { [SP.attack]: 0.0001, [SP.decay]: 0.001, [SP.release]: 0.001 }
const knobSeconds = (id: number, v: number) => Math.max(TIME_FLOOR_S[id] ?? 0, v * v * (TIME_MAX_S[id] ?? 1))
/** The pitch knob reaches this many semitones each way. */
const PITCH_RANGE_ST = 24

/** What `get_sampler_sample` says about the channel's sample. */
interface SamplerSampleInfo {
  path: string
  frames: number
  sampleRate: number
  channels: number
  root: number
  peaks: number[][]
}

function SamplerWindow({ trackId, slotId }: { trackId: string; slotId: string }) {
  const tracks = useTrackStore((s) => s.tracks)
  const bpm = useTransportStore((s) => s.bpm)
  const track = tracks.find((t) => t.id === trackId)
  const [page, setPage] = useState<Page>('sample')
  const [range, setRange] = useState(2)

  // The sampler's settings, 0..1 by parameter id, and their defaults.
  const [params, setParams] = useState<Record<number, number> | null>(null)
  const [defaults, setDefaults] = useState<Record<number, number>>({})
  useEffect(() => {
    let live = true
    invoke<PluginParam[]>('get_plugin_parameters', { trackId, slotId })
      .then((list) => {
        if (!live) return
        const v: Record<number, number> = {}
        const d: Record<number, number> = {}
        list.forEach((q) => { v[q.id] = q.value; d[q.id] = q.defaultValue })
        setParams(v)
        setDefaults(d)
      })
      .catch(() => { if (live) setParams({}) })
    return () => { live = false }
  }, [trackId, slotId])

  // The sample: its file, its length, its outline. Read again after a
  // load or a new root note.
  const [sample, setSample] = useState<SamplerSampleInfo | null | undefined>(undefined)
  const [sampleGen, setSampleGen] = useState(0)
  useEffect(() => {
    let live = true
    invoke<SamplerSampleInfo | null>('get_sampler_sample', { trackId, slotId, numBuckets: 700 })
      .then((s) => { if (live) setSample(s) })
      .catch(() => { if (live) setSample(null) })
    return () => { live = false }
  }, [trackId, slotId, sampleGen])

  // A change: shown at once, sent to the engine. A gesture's first move
  // takes the undo snapshot; its end names the step.
  const gesture = useRef(false)
  const sendTimer = useRef<number | null>(null)
  const pending = useRef(new Map<number, number>())
  const flush = (snapshot: boolean) => {
    const batch = [...pending.current]
    pending.current.clear()
    if (sendTimer.current !== null) { window.clearTimeout(sendTimer.current); sendTimer.current = null }
    return Promise.all(batch.map(([paramId, value]) =>
      invokeOrToast('set_plugin_parameter', { trackId, slotId, paramId, value, snapshot }, { message: 'Changing the sampler failed', retry: false }),
    ))
  }
  const set = (id: number, value: number, done: boolean, label: string) => {
    const same = params !== null && params[id] === value
    if (done && !gesture.current && same) return
    setParams((prev) => (prev ? { ...prev, [id]: value } : prev))
    pending.current.set(id, value)
    const first = !gesture.current
    if (!done) {
      gesture.current = true
      if (first) { void flush(true); return }
      if (sendTimer.current === null) sendTimer.current = window.setTimeout(() => { void flush(false) }, 40)
      return
    }
    gesture.current = false
    void flush(first).then(() => {
      useProjectStore.getState().markDirty()
      useHistoryStore.getState().push(label)
      if (id === SP.root) setSampleGen((g) => g + 1)
    })
  }
  const p = (id: number) => params?.[id] ?? defaults[id] ?? 0
  const def = (id: number) => defaults[id] ?? 0
  const on = (id: number) => p(id) >= 0.5
  const toggle = (id: number, label: string) => set(id, on(id) ? 0 : 1, true, label)
  const seconds = sample ? sample.frames / Math.max(1, sample.sampleRate) : 0

  const loadFile = async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const selected = await open({
      filters: [{ name: 'Audio', extensions: ['wav', 'flac', 'mp3', 'ogg', 'aiff', 'aif'] }],
      multiple: false,
    })
    if (!selected) return
    await invokeOrToast('load_sampler', { trackId, slotId, path: selected as string, baseNote: Math.round(p(SP.root) * 127) }, { message: 'Loading the sample failed', retry: false })
    useProjectStore.getState().markDirty()
    useHistoryStore.getState().push('Load sample')
    setSampleGen((g) => g + 1)
  }

  if (!track || params === null) return null

  const color = track.color || '#10b981'
  const fileName = sample?.path ? baseName(sample.path) : sample === undefined ? '' : 'No sample yet'
  const pitchSt = (p(SP.pitch) * 2 - 1) * PITCH_RANGE_ST
  const root = Math.round(p(SP.root) * 127)
  const loop = on(SP.loop)
  const loopHint = 'Switch on "Use loop points" first'
  const timeKnob = (id: number, label: string, done: string) => (
    <FlatKnob
      key={id} label={label} value={p(id)} def={def(id)}
      text={`${label} ${ms(knobSeconds(id, p(id)) * 1000)}`}
      onChange={(t, d) => set(id, Math.round(t * 1000) / 1000, d, done)}
      onReset={() => set(id, def(id), true, done)}
    />
  )

  return (
    <Frame
      name={track.name} color={color} hasEnvelope page={page} onPage={setPage}
      top={(
        <>
          <FlatSwitch label="On" on={!track.muted} onToggle={() => void useTrackStore.getState().toggleMute(trackId)} />
          <FlatKnob
            label="Pan" size={32} value={(track.pan + 1) / 2} def={0.5}
            text={`Pan ${track.pan === 0 ? 'centred' : `${Math.round(Math.abs(track.pan) * 100)}% ${track.pan < 0 ? 'left' : 'right'}`}`}
            onChange={(t, done) => {
              const pan = Math.round((t * 2 - 1) * 100) / 100
              const store = useTrackStore.getState()
              if (done) void store.setPan(trackId, pan)
              else store.setPanLocal(trackId, pan)
            }}
            onReset={() => void useTrackStore.getState().setPan(trackId, 0)}
          />
          <FlatKnob
            label="Vol" size={32} value={p(SP.gain)} def={def(SP.gain)}
            text={`Volume ${pct(p(SP.gain))}`}
            onChange={(t, done) => set(SP.gain, Math.round(t * 1000) / 1000, done, 'Channel volume')}
            onReset={() => set(SP.gain, def(SP.gain), true, 'Channel volume')}
          />
          <FlatKnob
            label="Pitch" size={32} value={(pitchSt + range) / (2 * range)} def={0.5}
            text={`Pitch ${pitchSt > 0 ? '+' : ''}${pitchSt.toFixed(2)} st`}
            onChange={(t, done) => {
              const st = Math.max(-PITCH_RANGE_ST, Math.min(PITCH_RANGE_ST, Math.round((t * 2 - 1) * range * 100) / 100))
              set(SP.pitch, st / PITCH_RANGE_ST / 2 + 0.5, done, 'Channel pitch')
            }}
            onReset={() => set(SP.pitch, 0.5, true, 'Channel pitch')}
          />
          <FlatBox label="Range" value={range} onStep={(by) => setRange((r) => Math.max(1, Math.min(PITCH_RANGE_ST, r + by)))} hint={`The pitch knob turns ${range} semitones each way. Scroll to change.`} />
          <TrackBox track={track} tracks={tracks} />
        </>
      )}
    >
      <div className={`sw-page${page === 'sample' ? '' : ' is-hidden'}`}>
        <div className="sw-cols">
          <div className="sw-col">
            <FlatPanel>
              <div className="sw-file">
                <span className="hwf-lab" style={{ textTransform: 'none', fontSize: 12 }}>File</span>
                <span className="sw-file-name" title={sample?.path || ''}>{fileName}</span>
                <span className="sw-grow" />
                <button type="button" className="sw-btn" onClick={() => void loadFile()} title="Load a sample into this channel">Load…</button>
              </div>
            </FlatPanel>
            <ContentPanel />
            <FlatPanel title="Playback" grow>
              <div className="hwf-krow">
                <FlatKnob
                  label="Offset" value={p(SP.offset)} def={def(SP.offset)}
                  text={`Start offset ${pct(p(SP.offset))}${seconds ? ` (${ms(p(SP.offset) * seconds * 1000)})` : ''}`}
                  onChange={(t, done) => set(SP.offset, Math.round(t * 1000) / 1000, done, 'Sample offset')}
                  onReset={() => set(SP.offset, 0, true, 'Sample offset')}
                />
                <FlatKnob
                  label="Loop start" value={p(SP.loopStart)} def={def(SP.loopStart)} disabled={!loop} offText={loopHint}
                  text={`Loop start ${seconds ? ms(p(SP.loopStart) * seconds * 1000) : pct(p(SP.loopStart))}`}
                  onChange={(t, done) => set(SP.loopStart, Math.round(Math.min(t, p(SP.loopEnd)) * 1000) / 1000, done, 'Loop start')}
                  onReset={() => set(SP.loopStart, 0, true, 'Loop start')}
                />
                <FlatKnob
                  label="Loop end" value={p(SP.loopEnd)} def={def(SP.loopEnd)} disabled={!loop} offText={loopHint}
                  text={`Loop end ${seconds ? ms(p(SP.loopEnd) * seconds * 1000) : pct(p(SP.loopEnd))}`}
                  onChange={(t, done) => set(SP.loopEnd, Math.round(Math.max(t, p(SP.loopStart)) * 1000) / 1000, done, 'Loop end')}
                  onReset={() => set(SP.loopEnd, 1, true, 'Loop end')}
                />
              </div>
              <div className="hwf-checks">
                <FlatCheck label="Use loop points" on={loop} onToggle={() => toggle(SP.loop, loop ? 'Loop off' : 'Loop on')} />
                <FlatCheck label="Ping pong loop" on={on(SP.pingPong)} disabled={!loop} hint={loop ? undefined : loopHint} onToggle={() => toggle(SP.pingPong, 'Ping pong loop')} />
              </div>
            </FlatPanel>
          </div>

          <div className="sw-col">
            <FlatPanel title="Time stretching">
              <div className="sw-row" style={{ alignItems: 'flex-start' }}>
                <div className="hwf-krow" style={{ flex: '0 0 192px' }}>
                  <FlatKnob label="Pitch" value={0.5} def={0.5} text="" disabled offText={FOR_CLIPS} />
                  <FlatKnob label="Mul" value={0.5} def={0.5} text="" disabled offText={FOR_CLIPS} />
                  <FlatKnob label="Time" value={0} def={0} text="" disabled offText={FOR_CLIPS} />
                </div>
                <div className="sw-stack">
                  <span className="hwf-lab">Mode</span>
                  <FlatSelect value="Resample" options={MODES} disabled />
                  <span className="sw-hint">{FOR_CLIPS}</span>
                </div>
              </div>
            </FlatPanel>
            <FlatPanel title="Precomputed effects" grow>
              <div className="hwf-checks">
                <FlatCheck label="Remove DC offset" on={false} disabled hint={NOT_YET} />
                <FlatCheck label="Reverse polarity" on={false} disabled hint={NOT_YET} />
                <FlatCheck label="Normalize" on={false} disabled hint={NOT_YET} />
                <FlatCheck label="Fade stereo" on={false} disabled hint={NOT_YET} />
                <FlatCheck label="Reverse" on={false} disabled hint={NOT_YET} />
                <FlatCheck label="Swap stereo" on={false} disabled hint={NOT_YET} />
              </div>
              <div className="hwf-krow">
                <FlatKnob label="Start" value={0} def={0} text="" disabled />
                <FlatKnob label="Length" value={1} def={1} text="" disabled />
                <FlatKnob label="In" value={0} def={0} text="" disabled />
                <FlatKnob label="Out" value={0} def={0} text="" disabled />
                <FlatKnob label="X-fade" value={0} def={0} text="" disabled />
                <FlatKnob label="Trim" value={0} def={0} text="" disabled />
              </div>
            </FlatPanel>
          </div>
        </div>

        <SampleView
          visible={page === 'sample'}
          peaks={sample?.peaks ?? []} seconds={seconds} bpm={bpm} color={color}
          region={{ start: p(SP.offset) * seconds, end: seconds }}
          loop={loop ? { start: p(SP.loopStart) * seconds, end: p(SP.loopEnd) * seconds } : null}
          fadeIn={0} fadeOut={0} reversed={false}
          empty={sample === null ? 'No sample yet: load one with the button above' : sample === undefined ? 'Loading…' : null}
        />
      </div>

      <div className={`sw-page${page === 'envelope' ? '' : ' is-hidden'}`}>
        <div className="sw-cols" style={{ gridTemplateColumns: '1fr' }}>
          <FlatPanel title="Volume envelope">
            <div className="sw-row">
              <div className="sw-targets" role="tablist">
                <button type="button" className="is-on">VOL</button>
                {['PAN', 'CUT', 'RES', 'PITCH'].map((t) => <button type="button" key={t} disabled title={`${t} envelope: ${NOT_YET.toLowerCase()}`}>{t}</button>)}
              </div>
              <span className="sw-grow" />
              <FlatCheck label="Envelope on" on={on(SP.envelope)} onToggle={() => toggle(SP.envelope, on(SP.envelope) ? 'Envelope off' : 'Envelope on')} />
            </div>
            <EnvelopeView
              visible={page === 'envelope'} color={color} on={on(SP.envelope)}
              delay={knobSeconds(SP.delay, p(SP.delay))} attack={knobSeconds(SP.attack, p(SP.attack))} hold={knobSeconds(SP.hold, p(SP.hold))}
              decay={knobSeconds(SP.decay, p(SP.decay))} sustain={p(SP.sustain)} release={knobSeconds(SP.release, p(SP.release))}
            />
            <div className="hwf-krow">
              {timeKnob(SP.delay, 'Delay', 'Envelope delay')}
              {timeKnob(SP.attack, 'Attack', 'Envelope attack')}
              {timeKnob(SP.hold, 'Hold', 'Envelope hold')}
              {timeKnob(SP.decay, 'Decay', 'Envelope decay')}
              <FlatKnob
                label="Sustain" value={p(SP.sustain)} def={def(SP.sustain)}
                text={`Sustain ${pct(p(SP.sustain))}`}
                onChange={(t, done) => set(SP.sustain, Math.round(t * 1000) / 1000, done, 'Envelope sustain')}
                onReset={() => set(SP.sustain, def(SP.sustain), true, 'Envelope sustain')}
              />
              {timeKnob(SP.release, 'Release', 'Envelope release')}
              <FlatKnob label="Amount" value={1} def={1} text="" disabled />
            </div>
          </FlatPanel>
        </div>
        <div className="sw-cols is-two">
          <FlatPanel title="Playing">
            <div className="hwf-checks" style={{ gridTemplateColumns: '1fr 1fr 1fr' }}>
              <FlatCheck label="Mono" on={on(SP.mono)} onToggle={() => toggle(SP.mono, on(SP.mono) ? 'Polyphonic' : 'Mono')} />
              <FlatCheck label="Cut itself" on={on(SP.cutItself)} onToggle={() => toggle(SP.cutItself, 'Cut itself')} />
              <FlatCheck label="Porta" on={on(SP.porta)} onToggle={() => toggle(SP.porta, 'Porta')} />
            </div>
          </FlatPanel>
          <FlatPanel title="Root note">
            <div className="sw-row">
              <span className="sw-hint">Plays the sample as recorded on</span>
              <span className="sw-grow" />
              <FlatSelect
                value={NOTE_OPTIONS[root]} options={NOTE_OPTIONS} style={{ flex: '0 0 84px' }}
                onPick={(n) => set(SP.root, NOTE_OPTIONS.indexOf(n) / 127, true, 'Root note')}
              />
            </div>
          </FlatPanel>
        </div>
      </div>
    </Frame>
  )
}

// ------------------------------------------------------------------ the pictures

/** The sample, wide at the bottom: what plays of it, its fades, its loop,
 *  the song's bars over it, and an overview strip under it. */
function SampleView({ peaks, seconds, bpm, color, region, loop, fadeIn, fadeOut, reversed, empty, visible = true }: {
  peaks: number[][]
  seconds: number
  bpm: number
  color: string
  region: { start: number; end: number }
  loop: { start: number; end: number } | null
  fadeIn: number
  fadeOut: number
  reversed: boolean
  /** What to say instead of a wave, when there is none to draw. */
  empty?: string | null
  /** A hidden page has no size to draw in; it draws when it is shown. */
  visible?: boolean
}) {
  const cv = useRef<HTMLCanvasElement>(null)
  const overview = useRef<HTMLCanvasElement>(null)
  const loopKey = loop ? `${loop.start}/${loop.end}` : ''

  useLayoutEffect(() => {
    const c = cv.current
    if (!c || !visible) return
    const dpr = window.devicePixelRatio || 1
    const w = c.clientWidth
    const h = c.clientHeight
    c.width = w * dpr
    c.height = h * dpr
    const g = c.getContext('2d')
    if (!g) return
    g.scale(dpr, dpr)
    const ruler = 18
    const total = seconds || Math.max(region.end, 1)
    const X = (t: number) => (t / total) * w
    g.fillStyle = '#0e0e10'; g.fillRect(0, 0, w, ruler)
    g.fillStyle = '#0a0a0b'; g.fillRect(0, ruler, w, h - ruler)
    g.fillStyle = 'rgba(255,255,255,.08)'; g.fillRect(0, ruler - 1, w, 1)
    const beat = 60 / Math.max(1, bpm)
    g.font = '600 11px Inter, sans-serif'
    for (let t = 0, b = 0; t < total; t += beat, b++) {
      const x = Math.round(X(t)) + 0.5
      g.strokeStyle = b % 4 === 0 ? 'rgba(255,255,255,.08)' : 'rgba(255,255,255,.035)'
      g.beginPath(); g.moveTo(x, ruler); g.lineTo(x, h); g.stroke()
      if (b % 4 === 0) { g.fillStyle = '#74747e'; g.fillText(String(b / 4 + 1), x + 4, 13) }
    }
    const mid = ruler + (h - ruler) / 2
    const half = (h - ruler) / 2 - 4
    const n = peaks.length
    for (let i = 0; i < n; i++) {
      const p = reversed ? peaks[n - 1 - i] : peaks[i]
      const t = ((i + 0.5) / n) * total
      const inside = t >= region.start && t <= region.end
      g.fillStyle = inside ? color : 'rgba(168,168,178,.18)'
      g.globalAlpha = inside ? 0.9 : 1
      const top = mid - Math.max(0, p[1]) * half
      const bottom = mid - Math.min(0, p[0]) * half
      g.fillRect((i / n) * w, top, Math.max(1, w / n + 0.4), Math.max(1, bottom - top))
    }
    g.globalAlpha = 1
    g.fillStyle = 'rgba(10,10,11,.6)'
    g.fillRect(0, ruler, X(region.start), h - ruler)
    g.fillRect(X(region.end), ruler, w - X(region.end), h - ruler)
    if (loop && loop.end > loop.start) {
      // The loop in red, as FL draws it: its span tinted, its ends marked.
      g.fillStyle = 'rgba(220,38,38,.12)'
      g.fillRect(X(loop.start), ruler, X(loop.end) - X(loop.start), h - ruler)
      g.fillStyle = '#dc2626'
      for (const t of [loop.start, loop.end]) {
        const x = Math.round(X(t)) + 0.5
        g.fillRect(x - 0.5, ruler, 1.5, h - ruler)
      }
      g.font = '600 10px Inter, sans-serif'
      g.fillText('LOOP', Math.min(X(loop.start) + 6, w - 34), h - 6)
    }
    g.strokeStyle = 'rgba(236,236,240,.7)'; g.lineWidth = 1.2
    if (fadeIn > 0) { g.beginPath(); g.moveTo(X(region.start), h); g.lineTo(X(region.start + fadeIn), ruler); g.stroke() }
    if (fadeOut > 0) { g.beginPath(); g.moveTo(X(region.end - fadeOut), ruler); g.lineTo(X(region.end), h); g.stroke() }
    g.lineWidth = 1
    const sx = Math.round(X(region.start)) + 0.5
    g.fillStyle = '#ececf0'
    g.fillRect(sx - 0.5, ruler, 1.5, h - ruler)
    g.beginPath(); g.moveTo(sx - 0.5, ruler); g.lineTo(sx + 7, ruler); g.lineTo(sx - 0.5, ruler + 8); g.closePath(); g.fill()
    g.font = '600 11px Inter, sans-serif'
    g.fillText('Start', sx + 10, ruler + 12)
    if (seconds) {
      g.fillStyle = '#74747e'; g.font = '500 11px Inter, sans-serif'; g.textAlign = 'right'
      g.fillText(`${(seconds / beat).toFixed(1)} beats at ${Math.round(bpm)} BPM`, w - 10, 13)
      g.textAlign = 'left'
    }
    if (!n) { g.fillStyle = '#74747e'; g.font = '500 12px Inter, sans-serif'; g.fillText(empty ?? 'The file is not loaded', 12, mid + 4) }

    const o = overview.current
    if (!o) return
    const ow = o.clientWidth
    const oh = o.clientHeight
    o.width = ow * dpr
    o.height = oh * dpr
    const og = o.getContext('2d')
    if (!og) return
    og.scale(dpr, dpr)
    og.fillStyle = '#1b1b20'; og.fillRect(0, 4, ow, oh - 8)
    og.fillStyle = color; og.globalAlpha = 0.55
    for (let i = 0; i < n; i++) {
      const a = Math.max(Math.abs(peaks[i][0]), Math.abs(peaks[i][1])) * (oh / 2 - 5)
      og.fillRect((i / n) * ow, oh / 2 - a, Math.max(1, ow / n), a * 2)
    }
  }, [peaks, seconds, bpm, color, region.start, region.end, loopKey, fadeIn, fadeOut, reversed, empty, visible]) // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="sw-view">
      <canvas ref={cv} className="sw-wave" />
      <canvas ref={overview} className="sw-overview" />
    </div>
  )
}

/** The volume envelope: delay, attack, hold, decay down to the sustain,
 *  the key let go, release. Dim when it is switched off. */
function EnvelopeView({ visible, color, on, delay, attack, hold, decay, sustain, release }: {
  visible: boolean; color: string; on: boolean; delay: number; attack: number; hold: number; decay: number; sustain: number; release: number
}) {
  const cv = useRef<HTMLCanvasElement>(null)
  useLayoutEffect(() => {
    const c = cv.current
    // A hidden page has no size to draw in; it draws when it is shown.
    if (!c || !visible) return
    const dpr = window.devicePixelRatio || 1
    const w = c.clientWidth
    const h = c.clientHeight
    c.width = w * dpr
    c.height = h * dpr
    const g = c.getContext('2d')
    if (!g) return
    g.scale(dpr, dpr)
    // The held part is drawn as a fixed share so the shape reads at any size.
    const heldFor = Math.max(0.05, (delay + attack + hold + decay + release) * 0.25)
    const total = delay + attack + hold + decay + heldFor + release
    const X = (t: number) => 14 + (t / total) * (w - 28)
    const Y = (v: number) => h - 14 - v * (h - 28)
    g.strokeStyle = 'rgba(255,255,255,.05)'
    for (let i = 1; i < 5; i++) { g.beginPath(); g.moveTo(0, (i * h) / 5 + 0.5); g.lineTo(w, (i * h) / 5 + 0.5); g.stroke() }
    const t1 = delay, t2 = t1 + attack, t3 = t2 + hold, t4 = t3 + decay, t5 = t4 + heldFor
    const pts: [number, number][] = [[0, 0], [t1, 0], [t2, 1], [t3, 1], [t4, sustain], [t5, sustain], [total, 0]]
    g.beginPath()
    pts.forEach(([t, v], i) => (i ? g.lineTo(X(t), Y(v)) : g.moveTo(X(t), Y(v))))
    g.lineTo(X(total), h); g.lineTo(X(0), h); g.closePath()
    g.fillStyle = color; g.globalAlpha = on ? 0.18 : 0.05; g.fill(); g.globalAlpha = 1
    g.beginPath()
    pts.forEach(([t, v], i) => (i ? g.lineTo(X(t), Y(v)) : g.moveTo(X(t), Y(v))))
    g.strokeStyle = on ? color : '#74747e'; g.lineWidth = 2; g.stroke(); g.lineWidth = 1
    for (const [t, v] of pts.slice(1, -1)) { g.fillStyle = '#fff'; g.beginPath(); g.arc(X(t), Y(v), 3.2, 0, Math.PI * 2); g.fill() }
    const off = X(t5)
    g.setLineDash([3, 4]); g.strokeStyle = 'rgba(255,255,255,.25)'
    g.beginPath(); g.moveTo(off, 8); g.lineTo(off, h); g.stroke(); g.setLineDash([])
    g.fillStyle = '#74747e'; g.font = '500 11px Inter, sans-serif'
    if (off + 84 > w) { g.textAlign = 'right'; g.fillText('key released', off - 6, 16); g.textAlign = 'left' } else g.fillText('key released', off + 6, 16)
    if (!on) { g.textAlign = 'right'; g.fillText('off: the sample plays as it is', w - 10, h - 8); g.textAlign = 'left' }
  }, [visible, color, on, delay, attack, hold, decay, sustain, release])
  return <div className="sw-env"><canvas ref={cv} /></div>
}
