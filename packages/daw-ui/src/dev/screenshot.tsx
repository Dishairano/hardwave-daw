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
import '../fonts'
import '../mockup.css' // the real top bar uses the fl-* classes from here
import '../rework.css'

import React from 'react'
import ReactDOM from 'react-dom/client'
import { HwTopbar, HwSecondRow, HwPlaylistTracks } from '../components/HwApp'
import { usePlaylistScrollStore } from '../stores/playlistScrollStore'
import { MacroPanel } from '../components/MacroPanel'
import { PresetBrowser } from '../components/PresetBrowser'
import { MidiFxPanel } from '../components/MidiFxPanel'
import { ManualWindow } from '../components/ManualWindow'
import { ModulationPanel } from '../components/ModulationPanel'
import { ReferencePanel } from '../components/ReferencePanel'
import { ScriptsPanel } from '../components/ScriptsPanel'
import { SpectralEditor } from '../components/arrangement/SpectralEditor'
import { SessionView } from '../components/SessionView'
import { CollabPanel } from '../components/CollabPanel'
import { WorkspaceSongs } from '../components/WorkspaceSongs'
import { TemplateDialog } from '../components/TemplateDialog'
import { PerformanceTest } from '../components/PerformanceTest'
import { StemsDialog } from '../components/arrangement/StemsDialog'
import { useCollabStore } from '../stores/collabStore'
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
import { useMidiCcStore } from '../stores/midiCcStore'
import { FloatingWindow, DetachButton } from '../components/FloatingWindow'
import { AudioSettings, focusSettingsTab, type SettingsTab } from '../components/settings/AudioSettings'
import { HwPluginWindow } from '../components/plugins/HwPluginWindow'

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
  // Three loop passes of a vocal, spread onto lanes the way comping
  // leaves them: the chosen take plays and the others are muted.
  track('t-vox', 'Vocal', '#f97316', [
    { ...clip('Take 1', 'src-vox-1', 0, 4), lane: 0 },
    { ...clip('Take 2', 'src-vox-2', 0, 4), lane: 1, muted: true },
    { ...clip('Take 3', 'src-vox-3', 0, 4), lane: 2, muted: true },
  ]),
  midiTrack,
]
// Plug-ins on the master, so the mixer's FX rack shows filled slots.
tracks[0].inserts = [
  { id: 'slot-eq', pluginId: 'hardwave.native.eq', pluginName: 'Hardwave EQ', enabled: true, wet: 1, sidechainSource: null },
  { id: 'slot-lim', pluginId: 'hardwave.native.limiter', pluginName: 'Hardwave Limiter', enabled: true, wet: 0.8, sidechainSource: null },
]
tracks[0].insert_count = 2

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
    // A built-in's window: ?id=eq (the layout key). ?menu=1 right-clicks
    // its first control, to photograph the parameter menu.
    case 'plugin': {
      const id = new URLSearchParams(location.search).get('id') || 'eq'
      if (new URLSearchParams(location.search).get('menu')) {
        setTimeout(() => {
          const el = document.querySelector('.hwp .kb')
          const r = el?.getBoundingClientRect()
          el?.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: (r?.left ?? 0) + 30, clientY: (r?.top ?? 0) + 30 }))
        }, 1200)
      }
      return (
        <div style={{ width: '100%', minHeight: '100%', background: '#08080c', display: 'flex', justifyContent: 'center', alignItems: 'flex-start', padding: 24, boxSizing: 'border-box' }}>
          <HwPluginWindow trackId="mock" slotId={`hardwave.native.${id}`} pluginId={`hardwave.native.${id}`} pluginName={id.replace(/_/g, ' ').replace(/^./, (c) => c.toUpperCase())} onClose={() => {}} />
        </div>
      )
    }
    case 'mixer':
      return <Full><MixerPanel /></Full>
    case 'channelrack':
      return <Full><ChannelRack /></Full>
    case 'pianoroll': {
      // SHOT_CC=cc1 opens a controller lane under the notes.
      const lane = new URLSearchParams(location.search).get('cc')
      if (lane) {
        const clipId = useTrackStore.getState().activeMidiClipId
        if (clipId) useMidiCcStore.getState().addLane(clipId, lane)
      }
      return <Full><PianoRoll /></Full>
    }
    case 'reference':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <ReferencePanel onClose={noop} />
        </div>
      )
    case 'scripts':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <ScriptsPanel onClose={noop} />
        </div>
      )
    case 'presence': {
      // The arrangement as one person sees it while the other works
      // at bar 5.
      useCollabStore.setState({
        connected: true,
        members: ['Dishaion', 'Alex'],
        peer: { name: 'Alex', tick: 4 * 4 * 960, trackIndex: 2, panel: 'Arrangement' },
      })
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
        </div>
      )
    }
    case 'stems':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Raw Drop">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <StemsDialog trackId="t" clipId="c" clipName="Vocal bounce" onClose={noop} />
        </div>
      )
    case 'perftest':
      // Press Start the way a person would, so the shot shows a result.
      setTimeout(() => {
        const start = [...document.querySelectorAll('button')].find(b => b.textContent === 'Start the test')
        start?.click()
        setTimeout(() => document.querySelector('details')?.setAttribute('open', ''), 100)
      }, 300)
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Raw Drop">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <PerformanceTest onClose={noop} onOpenTestSong={noop} />
        </div>
      )
    case 'newproject':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <TemplateDialog onPick={noop} onCancel={noop} />
        </div>
      )
    case 'workspace':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <WorkspaceSongs onClose={noop} />
        </div>
      )
    case 'collabhost':
      // The host, with someone at the door.
      ;(globalThis as { __hwCollabStatus?: unknown }).__hwCollabStatus = {
        connected: true, roomId: 'K7QM2XPA', inviteCode: 'R4T8WQ2NMZ', hosting: true, message: '',
        received: 0, sent: 0, members: ['Dishaion'], peer: null,
        joinRequest: { requestId: 'r1', name: 'Alex' }, waitingForHost: false,
      }
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Raw Drop">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <CollabPanel onClose={noop} />
        </div>
      )
    case 'collab':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <CollabPanel onClose={noop} />
        </div>
      )
    case 'session':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <SessionView onClose={noop} />
        </div>
      )
    case 'spectral':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <SpectralEditor trackId="track-1" clipId="clip-1" onClose={noop} />
        </div>
      )
    case 'modulation':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <ModulationPanel onClose={noop} />
        </div>
      )
    case 'manual':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <ManualWindow onClose={noop} />
        </div>
      )
    case 'midifx':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <MidiFxPanel trackId="t-lead" onClose={noop} />
        </div>
      )
    case 'presets':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <PresetBrowser onClose={noop} />
        </div>
      )
    case 'macros':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <MacroPanel onClose={noop} />
        </div>
      )
    case 'wizard':
      return <Full><WizardShot /></Full>
    case 'browser':
      return <Full><BrowserShot /></Full>
    case 'settings': {
      // SHOT_TAB=midi (?tab=) photographs a specific settings tab.
      focusSettingsTab((new URLSearchParams(location.search).get('tab') || 'audio') as SettingsTab)
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <FloatingWindow panelId="settings" title="Settings" dockable={false}
            actions={<DetachButton panelId="settings" />} onClose={noop}>
            <AudioSettings onClose={noop} />
          </FloatingWindow>
        </div>
      )
    }
    case 'playlistnames': {
      // The playlist as the app lays it out: names beside the grid, so
      // their rows can be checked against each other. &scrolly=N scrolls.
      const y = Number(new URLSearchParams(location.search).get('scrolly') || 0)
      if (y) setTimeout(() => usePlaylistScrollStore.getState().setY(y), 600)
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <div className="fl-playlist" style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <div className="fl-pl-body" style={{ ['--row-h' as never]: '56px' }}>
              <HwPlaylistTracks />
              <div className="fl-pl-grid">
                <div className="fl-pl-canvas">
                  <Arrangement onSetHint={noop} />
                </div>
              </div>
            </div>
          </div>
        </div>
      )
    }
    default:
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          {/* The REAL top bar (HwTopbar + HwSecondRow). */}
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
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
