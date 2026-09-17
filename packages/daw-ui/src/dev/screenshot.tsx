/**
 * Headless screenshot harness (dev-only). Renders real DAW panels against a
 * mocked Tauri backend + seeded stores so the UI can be captured with
 * Playwright (scripts/screenshot.mjs) — no Tauri runtime, no Windows machine.
 * Lets visual changes be verified before a build.
 *
 * Pick the panel with `?panel=` — playlist (default) | mixer | channelrack |
 * pianoroll | wizard | browser. Served only via screenshot.html on the vite dev server; the
 * production bundle (index.html → main.tsx) never imports this.
 */
import './tauri-mock' // must be first: installs window.__TAURI_INTERNALS__
import '../mockup.css' // the real top bar uses the fl-* classes from here

import React from 'react'
import ReactDOM from 'react-dom/client'
import { HwTopbar, HwSecondRow } from '../components/HwApp'
import { Arrangement } from '../components/arrangement/Arrangement'
import { ChannelRack } from '../components/channelrack/ChannelRack'
import { PianoRoll } from '../components/piano-roll/PianoRoll'
import { MixerPanel } from '../components/mixer/MixerPanel'
import { useTrackStore, type TrackWithClips, type ClipInfo } from '../stores/trackStore'
import { useTransportStore } from '../stores/transportStore'
import { useTempoMapStore } from '../stores/tempoMapStore'
import { meterSegments } from '../utils/meter'
import { SetupWizard } from '../components/SetupWizard'
import '../components/SetupWizard.css'
import { useSetupWizardStore, type WizardStep } from '../stores/setupWizardStore'
import { Browser } from '../components/browser/Browser'
import { useBrowserStore } from '../stores/browserStore'

/** Browser panel with a sample library added to Places and two folders open. */
function BrowserShot() {
  const [ready, setReady] = React.useState(false)
  React.useEffect(() => {
    const root = '/Users/producer/Samples'
    useBrowserStore.setState({
      diskRoots: [root],
      expandedDiskPaths: new Set([root, `${root}/Kicks`]),
    })
    setReady(true)
  }, [])
  return ready ? <Browser /> : null
}

/** Renders the SetupWizard opened on the audio step for UI screenshots. */
function WizardShot() {
  React.useEffect(() => {
    // ?step=velocity photographs a later step than the default.
    const step = (new URLSearchParams(location.search).get('step') ??
      'audio') as WizardStep
    useSetupWizardStore.setState({ visible: true, step })
  }, [])
  return <SetupWizard />
}

// ---- seed sample data -----------------------------------------------------

let clipSeq = 0
function clip(name: string, sourceId: string, posBars: number, lenBars: number, kind = 'Audio'): ClipInfo {
  const bar = 960 * 4
  return {
    id: `clip-${clipSeq++}`,
    name,
    kind,
    source_id: sourceId,
    position_ticks: posBars * bar,
    length_ticks: lenBars * bar,
    muted: false,
    gainDb: 0,
    fadeInTicks: 0,
    fadeOutTicks: 0,
    reversed: false,
    pitchSemitones: 0,
    stretchRatio: 1,
    fadeInCurve: 'linear' as ClipInfo['fadeInCurve'],
    fadeOutCurve: 'linear' as ClipInfo['fadeOutCurve'],
  }
}

function track(id: string, name: string, color: string, clips: ClipInfo[], kind = 'Audio'): TrackWithClips {
  return {
    id, name, kind, color,
    volume_db: kind === 'Master' ? 0 : -6 + Math.random() * 4,
    pan: 0, muted: false, soloed: false, armed: false, solo_safe: false,
    monitorInput: false, phaseInvert: false, swapLr: false, stereoSeparation: 0,
    delaySamples: 0, pitchSemitones: 0, fineTuneCents: 0,
    filterType: 'none', filterCutoffHz: 20000, filterResonance: 0,
    outputBus: null, insert_count: 0, inserts: [], automationLanes: [], automationClips: [],
    ...(kind === 'Midi' ? { instrument: 'builtin_sine' as never } : {}),
    clips,
  }
}

const midiTrack = track(
  't-lead', 'Lead', '#22c55e',
  [clip('Melody', 'src-lead', 0, 4, 'Midi'), clip('Melody', 'src-lead', 4, 4, 'Midi')],
  'Midi',
)

// An 8-bar "real session" — dense enough that marketing/gallery shots read
// like actual work, not an empty project.
const tracks: TrackWithClips[] = [
  track('t-master', 'Master', '#a1a1aa', [], 'Master'),
  track('t-kick', 'Induskick4', '#c9a227',
    Array.from({length: 8}, (_, i) => clip('Induskick4 – Auto', 'src-kick', i, 1))),
  track('t-crash', 'Crash #1', '#c026d3', [clip('Crash #1', 'src-crash', 0, 4), clip('Crash #1', 'src-crash', 4, 4)]),
  track('t-bass', 'Bass', '#2563eb',
    Array.from({length: 4}, (_, i) => clip('Reese', 'src-bass', i * 2, 2))),
  track('t-screech', 'Screech', '#ef4444', [clip('Screech', 'src-crash', 2, 2), clip('Screech', 'src-crash', 6, 2)]),
  track('t-fx', 'FX', '#14b8a6', [clip('Riser', 'src-crash', 3, 1), clip('Impact', 'src-kick', 4, 1), clip('Riser', 'src-crash', 7, 1)]),
  midiTrack,
]

const selected = tracks[2].clips[0].id // Crash — red selection header stands out
useTrackStore.setState({
  tracks,
  // The store keeps a derived id→track index; components like the mixer's
  // ChannelStrip read names via tracksById, so seed it too or strips show
  // the "Track" fallback.
  tracksById: Object.fromEntries(tracks.map((t) => [t.id, t])),
  selectedClipId: selected,
  selectedClipIds: new Set([selected]),
  // Open the MIDI clip so the piano roll renders with content.
  activeMidiTrackId: midiTrack.id,
  activeMidiClipId: midiTrack.clips[0].id,
} as never)

// ---- render ---------------------------------------------------------------

const noop = () => {}
const panel = new URLSearchParams(location.search).get('panel') || 'playlist'

// ?timesig=3 or ?timesig=7/8 seeds the project's signature, and
// ?timesigat=16:7/8 adds a change at beat 16, so a mid-song signature change
// can be seen rather than taken on trust.
const params = new URLSearchParams(location.search)

function parseSignature(text: string | null): { num: number; den: number } | null {
  if (!text) return null
  const [rawNum, rawDen] = text.split('/')
  const num = Number(rawNum)
  const den = rawDen === undefined ? 4 : Number(rawDen)
  if (!Number.isFinite(num) || num <= 0) return null
  if (!Number.isFinite(den) || den <= 0) return null
  return { num, den }
}

const first = parseSignature(params.get('timesig'))
if (first) {
  // Both: the store for the first paint, and the global the mocked backend
  // reads, or the transport poll would put 4/4 back.
  ;(window as unknown as { __HW_TIMESIG__?: number }).__HW_TIMESIG__ = first.num
  useTransportStore.setState({ timeSigNumerator: first.num, timeSigDenominator: first.den })
}

// The playlist draws its bars from the tempo map, so the map is what has to
// be seeded, not just the transport's read-out.
const entries = [{
  tick: 0,
  bpm: 140,
  timeSigNum: first?.num ?? 4,
  timeSigDen: first?.den ?? 4,
  ramp: 'instant',
}]
const changeParam = params.get('timesigat')
if (changeParam) {
  const [rawBeat, rawSig] = changeParam.split(':')
  const beat = Number(rawBeat)
  const changed = parseSignature(rawSig)
  if (Number.isFinite(beat) && beat > 0 && changed) {
    entries.push({
      tick: Math.round(beat * 960),
      bpm: 140,
      timeSigNum: changed.num,
      timeSigDen: changed.den,
      ramp: 'instant',
    })
  }
}
// The playlist re-reads the map at mount, so the mocked backend has to give
// the same answer as this seed.
;(window as unknown as { __HW_TEMPO_ENTRIES__?: unknown[] }).__HW_TEMPO_ENTRIES__ = entries
useTempoMapStore.setState({
  entries,
  segments: meterSegments(entries.map(e => ({
    tick: e.tick,
    timeSigNum: e.timeSigNum,
    timeSigDen: e.timeSigDen,
  }))),
})

function Full({ children }: { children: React.ReactNode }) {
  return <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>{children}</div>
}

function Harness() {
  switch (panel) {
    case 'mixer':
      return <Full><MixerPanel /></Full>
    case 'channelrack':
      return <Full><ChannelRack /></Full>
    case 'pianoroll':
      return <Full><PianoRoll /></Full>
    case 'wizard':
      return <Full><WizardShot /></Full>
    case 'browser':
      return <Full><BrowserShot /></Full>
    default:
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          {/* The REAL top bar (HwTopbar + HwSecondRow). */}
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} />
          <HwSecondRow projectName="Untitled" />
          <Arrangement onSetHint={noop} />
        </div>
      )
  }
}

ReactDOM.createRoot(document.getElementById('root')!).render(<Harness />)

// Async loads (waveform peaks, midi notes) resolve after mount and some draw
// effects key off horizontalZoom, not the load counter — nudge it so the
// static screenshot captures the loaded state.
setTimeout(() => {
  const z = useTransportStore.getState().horizontalZoom
  // SHOT_ZOOM (via ?zoom= in the harness URL) scales the playlist zoom so
  // marketing shots can fill the frame; default keeps the old 1.05 nudge.
  const mult = Number(new URLSearchParams(location.search).get('zoom')) || 1.05
  useTransportStore.setState({ horizontalZoom: z * mult })
}, 1200)
