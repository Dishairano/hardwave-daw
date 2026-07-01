/**
 * Headless screenshot harness (dev-only). Renders the real Toolbar +
 * Arrangement against a mocked Tauri backend and seeded stores so the UI can
 * be captured with Playwright (scripts/screenshot.mjs) — no Tauri runtime and
 * no Windows machine required. Lets visual changes be verified before a build.
 *
 * Served only via screenshot.html on the vite dev server; the production
 * bundle (index.html → main.tsx) never imports this.
 */
import './tauri-mock' // must be first: installs window.__TAURI_INTERNALS__

import React from 'react'
import ReactDOM from 'react-dom/client'
import { Toolbar } from '../components/transport/Toolbar'
import { Arrangement } from '../components/arrangement/Arrangement'
import { useTrackStore, type TrackWithClips, type ClipInfo } from '../stores/trackStore'
import { useTransportStore } from '../stores/transportStore'

// ---- seed sample data -----------------------------------------------------

let clipSeq = 0
function clip(name: string, sourceId: string, posBars: number, lenBars: number): ClipInfo {
  const PPQ = 960
  const bar = PPQ * 4
  return {
    id: `clip-${clipSeq++}`,
    name,
    kind: 'Audio',
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

function track(id: string, name: string, color: string, clips: ClipInfo[]): TrackWithClips {
  return {
    id,
    name,
    kind: 'Audio',
    color,
    volume_db: 0,
    pan: 0,
    muted: false,
    soloed: false,
    armed: false,
    solo_safe: false,
    monitorInput: false,
    phaseInvert: false,
    swapLr: false,
    stereoSeparation: 0,
    delaySamples: 0,
    pitchSemitones: 0,
    fineTuneCents: 0,
    filterType: 'none',
    filterCutoffHz: 20000,
    filterResonance: 0,
    outputBus: null,
    insert_count: 0,
    inserts: [],
    automationLanes: [],
    automationClips: [],
    clips,
  }
}

const tracks: TrackWithClips[] = [
  track('t-kick', 'Induskick4', '#c9a227', [
    clip('Induskick4 – Auto', 'src-kick', 0, 1),
    clip('Induskick4 – Auto', 'src-kick', 1, 1),
    clip('Induskick4 – Auto', 'src-kick', 2, 1),
    clip('Induskick4 – Auto', 'src-kick', 3, 1),
  ]),
  track('t-crash', 'Crash #1', '#c026d3', [clip('Crash #1', 'src-crash', 0, 4)]),
  track('t-bass', 'Bass', '#2563eb', [clip('Reese', 'src-bass', 0, 2), clip('Reese', 'src-bass', 2, 2)]),
]

// Select a clip on the Crash (green) track so the red selection header is
// visually distinct from the track's own colour.
const selected = tracks[1].clips[0].id
useTrackStore.setState({
  tracks,
  selectedClipId: selected,
  selectedClipIds: new Set([selected]),
})

// ---- render ---------------------------------------------------------------

const noop = () => {}

function Harness() {
  return (
    <div style={{ width: 1400, height: 560, display: 'flex', flexDirection: 'column', background: '#08080c' }}>
      <Toolbar
        showBrowser
        showPlaylist
        showChannelRack={false}
        showPianoRoll={false}
        showMixer={false}
        onToggleBrowser={noop}
        onTogglePlaylist={noop}
        onToggleChannelRack={noop}
        onTogglePianoRoll={noop}
        onToggleMixer={noop}
        onSetHint={noop}
      />
      {/* Arrangement's root is flex:1 — render it as a direct child of the
          flex column so it fills the remaining height (a non-flex wrapper
          would collapse it to 0 and the canvas would draw nothing). */}
      <Arrangement onSetHint={noop} />
    </div>
  )
}

ReactDOM.createRoot(document.getElementById('root')!).render(<Harness />)

// Waveform peaks load asynchronously and the canvas draw effect keys off
// horizontalZoom (not the internal load counter), so a static harness would
// screenshot before the waveform paints. Nudge the zoom once peaks are in to
// force one clean redraw with the spectral waveform present.
setTimeout(() => {
  const z = useTransportStore.getState().horizontalZoom
  useTransportStore.setState({ horizontalZoom: z * 1.05 })
}, 1200)
