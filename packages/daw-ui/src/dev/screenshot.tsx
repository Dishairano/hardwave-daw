/**
 * Headless screenshot harness (dev-only). Renders real DAW panels against a
 * mocked Tauri backend + seeded stores so the UI can be captured with
 * Playwright (scripts/screenshot.mjs) — no Tauri runtime, no Windows machine.
 * Lets visual changes be verified before a build.
 *
 * Pick the panel with `?panel=` — playlist (default) | mixer | channelrack |
 * pianoroll | firstrun | newcontroller | browser. Served only via screenshot.html on the vite dev server; the
 * production bundle (index.html → main.tsx) never imports this.
 */
import { MOCK_DRUM_FOLDER } from './tauri-mock' // must be first: installs window.__TAURI_INTERNALS__
import { flSongTracks } from './flSongSeed'
import '../fonts'
import '../mockup.css' // the real top bar uses the fl-* classes from here
import '../rework.css'

import React from 'react'
import { invoke } from '@tauri-apps/api/core'
import ReactDOM from 'react-dom/client'
import { HwApp, HwTopbar, HwSecondRow, HwPlaylistTracks, HwPlaylistTools } from '../components/HwApp'
import { HistoryPanel } from '../components/HistoryPanel'
import { LoudnessMeter } from '../components/LoudnessMeter'
import { Oscilloscope } from '../components/Oscilloscope'
import { SpectrumAnalyzer } from '../components/SpectrumAnalyzer'
import { MidiMappingsPanel } from '../components/MidiMappingsPanel'
import { TuneDialog } from '../components/arrangement/TuneDialog'
import { MidiRouteDialog } from '../components/arrangement/MidiRouteDialog'
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
import { usePlaylistRowsStore } from '../stores/playlistRowsStore'
import { useGeneralPrefsStore } from '../stores/generalPrefsStore'
import { useTransportStore } from '../stores/transportStore'
import { usePatternStore } from '../stores/patternStore'
import { useTempoMapStore } from '../stores/tempoMapStore'
import { meterSegments } from '../utils/meter'
import { FirstRunFlow } from '../components/firstrun/FirstRunFlow'
import { useSetupWizardStore, type WizardStep } from '../stores/setupWizardStore'
import { NotificationHost } from '../components/NotificationHost'
import { midiInputAppeared, type InputKind } from '../lib/midiHotplug'
import { Browser } from '../components/browser/Browser'
import { useBrowserStore } from '../stores/browserStore'
import { useMidiCcStore } from '../stores/midiCcStore'
import { FloatingWindow, DetachButton } from '../components/FloatingWindow'
import { AudioSettings, focusSettingsTab, type SettingsTab } from '../components/settings/AudioSettings'
import { HwPluginWindow } from '../components/plugins/HwPluginWindow'
import { SampleWindow } from '../components/sample-window/SampleWindow'
import { useSampleWindowStore } from '../stores/sampleWindowStore'
import { sampleAtTick } from '../utils/tempoTime'
import { bpmToY, laneScale } from '../components/arrangement/tempoLaneDraw'

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

/** The channel rack with a drum pattern in it, so the steps show. */
function RackShot() {
  React.useEffect(() => {
    const t = setTimeout(() => {
      const tracks = useTrackStore.getState().tracks
      const rows = [[0, 4, 8, 12], [0], [2, 6, 10, 14], [0, 3, 6, 8, 11, 14], [12, 13, 14, 15], [8, 9], [0, 2, 4, 6, 8, 10, 12, 14]]
      const vels = [1, 0.9, 0.8, 0.85, 0.6, 0.75, 0.55]
      const steps: Record<string, number[]> = {}
      tracks.forEach((tr, i) => {
        const on = rows[i % rows.length]
        steps[tr.id] = Array.from({ length: 16 }, (_, k) => (on.includes(k) ? (i % 7 === 6 ? (k % 4 === 0 ? 1 : 0.45) : vels[i % 7]) : 0))
      })
      const s = usePatternStore.getState()
      const activeId = s.activeId ?? s.patterns[0]?.id
      usePatternStore.setState({ patterns: s.patterns.map(p => (p.id === activeId ? { ...p, steps: { ...p.steps, ...steps } } : p)) })
    }, 300)
    return () => clearTimeout(t)
  }, [])
  return <ChannelRack />
}

/** A new song as it opens: Kick (the kick synth), Clap, Hat and Snare on the
 *  first rows, the samplers naming the drums installed with the app, and
 *  nothing open in the piano roll. */
function seedNewSong() {
  const sampler = (id: string, file: string) => [{
    id: `slot-${id}`, pluginId: 'hardwave.native.sampler', pluginName: 'Hardwave Sampler',
    enabled: true, wet: 1, sidechainSource: null, samplePath: `${MOCK_DRUM_FOLDER}/${file}`,
  }]
  const song: TrackWithClips[] = [
    track('master', 'Master', '#a1a1aa', [], 'Master'),
    { ...track('insert-001', 'Kick', '#DC2626', [], 'Midi'), instrument: 'kick_synth', volume_db: 0 },
    { ...track('insert-002', 'Clap', '#10B981', [], 'Midi'), inserts: sampler('clap', 'Clap 1.wav'), insert_count: 1, volume_db: -3 },
    { ...track('insert-003', 'Hat', '#A855F7', [], 'Midi'), inserts: sampler('hat', 'Hat 1.wav'), insert_count: 1, volume_db: -8 },
    { ...track('insert-004', 'Snare', '#F59E0B', [], 'Midi'), inserts: sampler('snare', 'Snare 1.wav'), insert_count: 1, volume_db: -3 },
    ...Array.from({ length: 6 }, (_, i) => track(`insert-${String(i + 5).padStart(3, '0')}`, `Insert ${i + 5}`, '#52525b', [])),
  ]
  numberAsBefore(song)
  useTrackStore.setState({
    tracks: song,
    tracksById: Object.fromEntries(song.map((t) => [t.id, t])),
    selectedTrackId: null, selectedClipId: null, selectedClipIds: new Set(),
    activeMidiTrackId: null, activeMidiClipId: null,
  } as never)
  ;(window as unknown as { __hwMockTracks?: unknown[] }).__hwMockTracks = JSON.parse(JSON.stringify(song))
  // What the plug-in scan found: the built-ins and a few free synths.
  const plugin = (id: string, name: string, vendor: string, format = 'Vst3') =>
    ({ id, name, vendor, version: '1', format, path: id.startsWith('hardwave.native.') ? '<native>' : `/plugins/${name}.vst3`, category: 'Instrument', has_midi_input: true, has_editor: true })
  ;(window as unknown as { __hwMockPlugins?: unknown[] }).__hwMockPlugins = [
    plugin('hardwave.native.tripleosc', 'Hardwave 3xOSC', 'Hardwave'),
    plugin('hardwave.native.wavetable', 'Hardwave Wavetable', 'Hardwave'),
    plugin('hardwave.native.fm', 'Hardwave FM', 'Hardwave'),
    plugin('hardwave.native.noise', 'Hardwave Noise', 'Hardwave'),
    plugin('hardwave.native.sampler', 'Hardwave Sampler', 'Hardwave'),
    plugin('vital', 'Vital', 'Vital Audio'),
    plugin('surge-xt', 'Surge XT', 'Surge Synth Team', 'Clap'),
    plugin('dexed', 'Dexed', 'Digital Suburban'),
    plugin('obxd', 'OB-Xd', 'discoDSP'),
  ]
}

/** The new song: the browser on the drums folder, the channel rack over the
 *  playlist, and the mouse resting on Clap's name (its tooltip). */
function NewSongShot() {
  React.useEffect(() => {
    const tip = setTimeout(() => {
      const clap = document.querySelectorAll('[data-testid="channel-name"]')[1] as HTMLElement | undefined
      const r = clap?.getBoundingClientRect()
      if (!clap || !r) return
      clap.dispatchEvent(new MouseEvent('mouseover', { bubbles: true, clientX: r.left + 60, clientY: r.top + 12 }))
    }, 900)
    return () => clearTimeout(tip)
  }, [])
  return (
    <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
      <HwTopbar showPlaylist showChannelRack showPianoRoll={false} showMixer={false} projectName="Untitled">
        <HwSecondRow />
      </HwTopbar>
      <div className="fl-body">
        <div className="fl-browser"><Browser /></div>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
          <div style={{ height: 230, flex: 'none', borderBottom: '1px solid rgba(255,255,255,0.07)' }}><ChannelRack /></div>
          <div className="fl-playlist" style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <div className="fl-pl-body" style={{ ['--row-h' as never]: '52px' }}>
              <HwPlaylistTracks />
              <div className="fl-pl-grid">
                <div className="fl-pl-canvas">
                  <Arrangement onSetHint={noop} />
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

/** The first run, opened on a step (?step=start | sound | controller |
 *  song), over the DAW as on a first launch. On the Controller step one
 *  controller is plugged in and C3 was just played; ?ports=none takes the
 *  controllers away, so the stepper shows Controller struck through. */
function FirstRunShot() {
  const params = new URLSearchParams(location.search)
  const step = (params.get('step') ?? 'start') as WizardStep
  const w = window as unknown as { __hwMidiPorts?: string[]; __hwLastNote?: unknown; __hwToneHold?: boolean }
  w.__hwMidiPorts = params.get('ports') === 'none' ? [] : ['Akai MPK Mini mk3']
  if (step === 'controller') w.__hwLastNote = { note: 48, velocity: 96 }
  if (step === 'sound') w.__hwToneHold = true
  React.useEffect(() => {
    useSetupWizardStore.setState({ visible: true, step, completedFirstRun: false, skippedAt: null, midiInputs: {} })
  }, [step])
  return <FirstRunFlow recentProjects={[]} onOpenRecent={noop} onOpenProject={noop} onStartSong={noop} />
}

/** A controller plugged in while a song is open: the message bottom right.
 *  ?kind=desk plugs in a control surface instead of a keyboard. */
function NewControllerShot() {
  React.useEffect(() => {
    const kind = (new URLSearchParams(location.search).get('kind') ?? 'keyboard') as InputKind
    const port = kind === 'desk' ? 'Behringer X-Touch' : 'Akai MPK Mini mk3'
    useSetupWizardStore.setState({ visible: false, midiInputs: {}, midiMasterEnabled: true })
    const t = setTimeout(() => { void midiInputAppeared({ port, kind }) }, 400)
    return () => clearTimeout(t)
  }, [])
  return <NotificationHost />
}

// ---- seed sample data -----------------------------------------------------

/** Number a seeded song the way a song made before FL's three lists opens
 *  (step 1's migration): the master 0, then every track that is not
 *  automation 1, 2, 3 in order; every track the rack listed before (not an
 *  "insert-NNN" row, or a MIDI one) in rack group 0. */
function numberAsBefore(song: TrackWithClips[]) {
  let next = 1
  for (const t of song) {
    t.mixerNumber = t.kind === 'Master' ? 0 : t.kind === 'Automation' ? null : next++
    t.rackGroup = t.kind !== 'Master' && t.kind !== 'Bus' && (!t.id.startsWith('insert-') || t.kind === 'Midi') ? 0 : null
  }
}

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

numberAsBefore(tracks)

/** The rows a song made before FL's rows has: each track's own row with
 *  its name and colour, then a row per automation lane and clip under it
 *  (what `Project::lay_out_as_before` writes). */
function rowsAsBefore(list: TrackWithClips[]) {
  const rows: { name: string; color: string | null; muted: boolean; soloed: boolean }[] = []
  for (const t of list) {
    if (t.kind === 'Master') continue
    rows.push({ name: t.name, color: t.color, muted: false, soloed: false })
    const under = t.automationLanes.length + (t.automationClips?.length ?? 0)
    for (let i = 0; i < under; i++) rows.push({ name: '', color: null, muted: false, soloed: false })
  }
  return rows
}
;(window as unknown as { __hwMockRows?: unknown[] }).__hwMockRows = rowsAsBefore(tracks)
usePlaylistRowsStore.setState({ rows: rowsAsBefore(tracks) as never })

/**
 * An FL song in the playlist's rows, the shape of mockup M1: unnamed rows
 * ("Track N") in FL's row colours, clips of several channels side by side
 * on one row, automation clips on the grid on rows of their own, and row
 * 11 switched off. &menu=N opens row N's menu, &tip=1 rests the pointer on
 * a clip of row 19.
 */
function seedFlRows() {
  const bar = 960 * 4
  const on = (c: ClipInfo, row: number): ClipInfo => ({ ...c, row })
  const seq = (name: string, src: string, row: number, from: number, to: number, len = 1, every = 1) =>
    Array.from({ length: Math.floor((to - from) / every) }, (_, i) => on(clip(name, src, from + i * every, len), row))
  const auto = (id: string, row: number, from: number, len: number, points: [number, number][], color: number) => ({
    id, target: { kind: 'track_volume' as const }, startTick: from * bar, lengthTicks: len * bar,
    colorArgb: color, row,
    points: points.map(([b, v]) => ({ tick: b * bar, value: v, curve: 'Linear', tension: 0 })),
  })
  const snare = track('ch-909', '909 Snare', '#e05a6d', [
    ...seq('909 Snare', 'src-kick', 18, 0, 16, 0.25, 0.5),
  ])
  snare.automationClips = [
    auto('ac-snare-vol', 3, 0, 7, [[0, 0.2], [7, 0.9]], 0xffb06a5a),
    auto('ac-snare-pitch', 2, 0, 8, [[0, 0.5], [6, 0.5], [8, 1]], 0xffa0609a),
  ]
  const kick = track('ch-kick', 'KIT_01_Kick_02', '#d6334f', [
    ...seq('KIT_01_Kick_02', 'src-kick', 18, 0.25, 16, 0.25, 0.5),
    ...seq('KIT_01_Kick_02', 'src-kick', 4, 8, 16, 1, 1),
  ])
  const melodic = track('ch-mel', 'TKNVLT_HGE1_MELODIC_LOOP_9', '#b8336a', [
    on(clip('TKNVLT_HGE1_MELODIC_LOOP_9', 'src-crash', 9, 4), 18),
    on(clip('TKNVLT_HGE1_MELODIC_LOOP_9', 'src-crash', 13, 3), 18),
  ])
  const drums = track('ch-drum', 'IT_Drum_Loop_15_Kick_Loop_135', '#c2185b', [
    ...seq('IT_Drum_Loop_15_Kick_Loop_135', 'src-kick', 5, 0, 16, 4, 4),
    on(clip('IT_Drum_Loop_15_Kick_Loop_135', 'src-kick', 0, 4), 1),
  ])
  const top = track('ch-top', 'TOP_LOOP_135BPM', '#7b1fa2', seq('TOP_LOOP_135BPM (2)', 'src-crash', 7, 4, 16, 4, 4))
  const attitude = track('ch-att', 'THM - Attitude - Push', '#ad1457', [
    on(clip('THM - Attitude - Push', 'src-vox-1', 4, 4), 9),
    on(clip('THM - Downlifter 14', 'src-vox-2', 8, 2), 10),
    on(clip('THM - White Noise Up', 'src-vox-3', 4, 3), 12),
  ])
  const bass = track('ch-bass', 'KIT_02_Bass_02_128BPM', '#c62828', [
    ...seq('KIT_02_Bass_02_128BPM', 'src-bass', 3, 8, 16, 2, 2),
    ...seq('KIT_02_Bass_02_128BPM', 'src-bass', 0, 0, 16, 1, 1),
  ])
  const master = track('master', 'Master', '#a1a1aa', [], 'Master')
  master.automationClips = [auto('ac-master-vol', 20, 12, 4, [[0, 1], [3, 1], [4, 0.6]], 0xff3f8a6a)]
  // A fader of its own for each channel, playing into numbered inserts.
  const song = [master, snare, kick, melodic, drums, top, attitude, bass].map((t, i) => (
    i === 0 ? { ...t, mixerNumber: 0 } : { ...t, rackGroup: 0, mixerNumber: [0, 18, 4, 7, 2, 9, 11, 3][i] }
  ))
  const colours = ['#a83c5a', '#a04060', '#8e3a6e', '#7c3a7a', '#a83c5a', '#9a3a5e', '#863a72', '#a83c5a', '#7c3a7a', '#9a3a5e', '#a83c5a',
    '#8e3a6e', '#a04060', '#7c3a7a', '#a83c5a', '#9a3a5e', '#863a72', '#a83c5a', '#a04060', '#7c3a7a', '#8e3a6e', '#a83c5a']
  const rows = colours.map((color, i) => ({ name: '', color, muted: i === 10, soloed: false }))
  useTrackStore.setState({
    tracks: song, tracksById: Object.fromEntries(song.map((t) => [t.id, t])),
    selectedClipId: null, selectedClipIds: new Set(),
  } as never)
  useTransportStore.setState({ bpm: 130, horizontalZoom: 0.55, trackHeight: 28 } as never)
  useGeneralPrefsStore.setState({ tempoLaneShown: false } as never)
  usePlaylistRowsStore.setState({ rows: rows as never, selectedRow: null })
  const w = window as unknown as { __hwMockTracks?: unknown[]; __hwMockRows?: unknown[] }
  w.__hwMockTracks = JSON.parse(JSON.stringify(song))
  w.__hwMockRows = rows
}

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

/** The playlist with one more dialog over it, for the dialog shots. */
function Over({ children, project = 'Untitled' }: { children: React.ReactNode; project?: string }) {
  return (
    <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
      <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName={project}>
        <HwSecondRow />
      </HwTopbar>
      <Arrangement onSetHint={noop} />
      {children}
    </div>
  )
}

/** The Tune and MIDI-route dialogs need a track: the first MIDI track, else the first. */
function TuneShot() {
  const t = useTrackStore(s => s.tracks.find(x => (x.kind || '').toLowerCase() === 'midi') ?? s.tracks[0])
  if (!t) return null
  return <TuneDialog trackId={t.id} clipId={t.clips[0]?.id ?? ''} onClose={noop} />
}
function MidiRouteShot() {
  const t = useTrackStore(s => s.tracks.find(x => (x.kind || '').toLowerCase() === 'midi') ?? s.tracks[0])
  if (!t) return null
  return <MidiRouteDialog trackId={t.id} onClose={noop} />
}
const panel = new URLSearchParams(location.search).get('panel') || 'playlist'

// ?timesig=3 or ?timesig=7/8 seeds the project's signature, and
// ?timesigat=16:7/8 adds a change at beat 16, so a mid-song signature change
// can be seen rather than taken on trust.
const params = new URLSearchParams(location.search)

// ?song=fl: the founder's FL song in FL's shape (mockups M2 and M3), for the
// mixer and channel rack shots.
if (params.get('song') === 'fl') {
  const song = flSongTracks()
  useTrackStore.setState({
    tracks: song,
    tracksById: Object.fromEntries(song.map((t) => [t.id, t])),
    selectedTrackId: null, selectedClipId: null, selectedClipIds: new Set(),
    activeMidiTrackId: null, activeMidiClipId: null,
  } as never)
  ;(window as unknown as { __hwMockTracks?: unknown[] }).__hwMockTracks = JSON.parse(JSON.stringify(song))
}

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
// panel=tempolane: the song the tempo lane mockup draws, 150 BPM ramping to
// 160 through the build (bars 9 to 17) and back to 150 for the drop.
if (params.get('panel') === 'tempolane') {
  const bar = 960 * 4
  entries.splice(0, entries.length,
    { tick: 0, bpm: 150, timeSigNum: 4, timeSigDen: 4, ramp: 'instant' },
    { tick: bar * 8, bpm: 150, timeSigNum: 4, timeSigDen: 4, ramp: 'linear' },
    { tick: bar * 16 - 1, bpm: 160, timeSigNum: 4, timeSigDen: 4, ramp: 'instant' },
    { tick: bar * 16, bpm: 150, timeSigNum: 4, timeSigDen: 4, ramp: 'instant' },
  )
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
      return <Full><RackShot /></Full>
    case 'newsong':
      seedNewSong()
      return <NewSongShot />
    case 'emptyroll':
      // The piano roll with nothing open in it, in a new song.
      seedNewSong()
      return <Full><PianoRoll /></Full>
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
    case 'history':
      return <Over><HistoryPanel onClose={noop} /></Over>
    case 'loudness':
      return <Over><LoudnessMeter onClose={noop} /></Over>
    case 'oscilloscope':
      return <Over><Oscilloscope onClose={noop} /></Over>
    case 'spectrum':
      return <Over><SpectrumAnalyzer onClose={noop} /></Over>
    case 'midimappings':
      return <Over><MidiMappingsPanel onClose={noop} /></Over>
    case 'tune':
      return <Over><TuneShot /></Over>
    case 'midiroute':
      return <Over><MidiRouteShot /></Over>
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
    // The account at the top right: ?account=pro (default) | free | trial |
    // cancelled | staff | signedout | ended | offline, and ?open=1 opens
    // its panel the way a click does.
    case 'account': {
      const which = params.get('account') || 'pro'
      const signedIn = (plan: unknown) => ({
        state: 'signedIn', name: 'Dishairano', email: 'dis…@hardwavestudios.com', plan,
      })
      const scenes: Record<string, unknown> = {
        pro: signedIn({ kind: 'pro', via: 'subscription', until: null }),
        trial: signedIn({ kind: 'pro', via: 'trial', until: null }),
        cancelled: signedIn({ kind: 'pro', via: 'subscription', until: '2026-12-12' }),
        staff: signedIn({ kind: 'pro', via: 'staff', until: null }),
        free: signedIn({ kind: 'free' }),
        signedout: { state: 'signedOut' },
        ended: { state: 'ended' },
        offline: { state: 'unreachable' },
      }
      ;(globalThis as { __hwAccount?: unknown }).__hwAccount = scenes[which] ?? scenes.pro
      if (params.get('open')) {
        setTimeout(() => (document.querySelector('.hw-acct') as HTMLElement | null)?.click(), 600)
      }
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Raw Drop">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
        </div>
      )
    }
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
    // The whole shell with the panels in the work area (moving, docking).
    case 'panels':
      return (
        <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column' }}>
          <HwApp
            showBrowser showPlaylist showChannelRack showPianoRoll={false} showMixer
            isMobile={false} mobilePanel="playlist"
            onToggleBrowser={noop} onTogglePlaylist={noop} onToggleChannelRack={noop} onTogglePianoRoll={noop} onToggleMixer={noop}
          />
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
    case 'firstrun':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <FirstRunShot />
        </div>
      )
    case 'newcontroller':
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Hardstyle starter">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <NewControllerShot />
        </div>
      )
    case 'browser':
      // Docked at its real width beside the playlist, as in the app.
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column' }}>
          <div className="fl-body">
            <div className="fl-browser"><BrowserShot /></div>
            <div style={{ flex: 1, display: 'flex', minWidth: 0 }}><Arrangement onSetHint={noop} /></div>
          </div>
        </div>
      )
    case 'settings': {
      // SHOT_TAB=midi (?tab=) photographs a specific settings tab.
      const tab = (new URLSearchParams(location.search).get('tab') || 'audio') as SettingsTab
      focusSettingsTab(tab)
      // ?playing=1: a song plays, so Apply does not play the tone over it.
      if (new URLSearchParams(location.search).get('playing')) useTransportStore.setState({ playing: true })
      // ?tone=1: just after Apply, the test tone on the right side.
      if (new URLSearchParams(location.search).get('tone')) {
        ;(window as unknown as { __hwToneHold?: boolean }).__hwToneHold = true
        setTimeout(() => {
          const button = [...document.querySelectorAll('button')].find(b => b.textContent?.includes('Test sound'))
          button?.click()
        }, 500)
      }
      // A keyboard and a desk plugged in, so the control surface's desk
      // input picker has both to show.
      if (tab === 'midi') {
        for (const portName of ['Novation Launchkey 49', 'X-Touch']) {
          void invoke('open_midi_input', { portName })
        }
      }
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
    case 'tempolane': {
      // The playlist with the tempo lane, as the mockup has it: a kick and a
      // screech in beats, a drum loop that follows the tempo and a vocal
      // that keeps its speed, the playhead at the drop. &lane=menu opens
      // the lane's menu at bar 4; &lane=drag drags the ramp's end.
      const bar = 960 * 4
      const audio = (name: string, src: string, at: number, bars: number, followBpm: number | null, secs: number): ClipInfo => ({
        ...clip(name, src, at, bars, 'audio'),
        sourceFrames: Math.round(secs * 48000), sourceRate: 48000, sourceStart: 0, timeFactor: 1,
        treatment: { pan: 0, polarityInverted: false, swapStereo: false, normalize: false, removeDc: false, resample: false, followBpm },
      })
      const laneTracks: TrackWithClips[] = [
        track('t-kick', 'Kick', '#e5243b', [clip('Kick', 'src-kick', 0, 24, 'midi')], 'Midi'),
        track('t-screech', 'Screech', '#f59e0b', [clip('Screech', 'src-lead', 8, 16, 'midi')], 'Midi'),
        track('t-loop', 'Drum loop', '#22c55e', [
          audio('Loop 150 BPM', 'src-loop', 0, 16, 150, 16 * 1.6),
          audio('Loop 150 BPM', 'src-loop', 16, 8, 150, 8 * 1.6),
        ]),
        // Eight bars at 150 as recorded: across the build it covers a
        // little more than eight bars, as the engine makes it.
        track('t-vox', 'Vocal', '#3b82f6', [{
          ...audio('Vocal - Rise up', 'src-vox-1', 12, 8, null, 12.8),
          length_ticks: Math.round(bar * 8.13),
        }]),
      ]
      if (!(window as unknown as { __hwLaneSeeded?: boolean }).__hwLaneSeeded) {
        ;(window as unknown as { __hwLaneSeeded?: boolean }).__hwLaneSeeded = true
        useTrackStore.setState({ tracks: laneTracks, tracksById: Object.fromEntries(laneTracks.map((t) => [t.id, t])), selectedClipId: null, selectedClipIds: new Set() } as never)
        ;(window as unknown as { __hwMockTracks?: unknown }).__hwMockTracks = laneTracks
        useTransportStore.setState({ horizontalZoom: 0.31, bpm: 150, positionSamples: sampleAtTick(entries, bar * 16, 48000) })
        const shot = params.get('lane')
        setTimeout(() => {
          const lane = document.querySelector('[data-testid="tempo-lane"]') as HTMLElement | null
          if (!lane) return
          const r = lane.getBoundingClientRect()
          const pxPerTick = (100 * useTransportStore.getState().horizontalZoom) / (150 / 60) / 960
          const scale = laneScale(entries)
          const at = (tick: number, bpm: number) => ({ clientX: r.left + tick * pxPerTick, clientY: r.top + bpmToY(bpm, scale) })
          if (shot === 'menu') {
            lane.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, ...at(bar * 3, 150) }))
          } else if (shot === 'drag') {
            const from = at(bar * 16 - 1, 160)
            lane.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, ...from }))
            setTimeout(() => {
              window.dispatchEvent(new MouseEvent('mousemove', { ...from, clientY: from.clientY - 4 }))
              window.dispatchEvent(new MouseEvent('mousemove', { ...from, clientY: from.clientY - 9 }))
            }, 100)
          }
        }, 1600)
      }
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Rawstyle anthem.hwp">
            <HwSecondRow />
          </HwTopbar>
          <div className="fl-playlist" style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <HwPlaylistTools />
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
    case 'flrows': {
      // Mockup M1: an FL song's rows. &menu=N opens row N's menu (1-based)
      // and &tip=1 rests the pointer on a clip of row 19.
      seedFlRows()
      const q = new URLSearchParams(location.search)
      const menuRow = Number(q.get('menu') || 0)
      setTimeout(() => {
        if (menuRow > 0) {
          const el = document.querySelector(`[data-row="${menuRow - 1}"]`) as HTMLElement | null
          const r = el?.getBoundingClientRect()
          if (el && r) el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: r.left + 40, clientY: r.top + r.height / 2 }))
        }
        if (q.get('tip')) {
          const canvas = document.querySelector('[data-testid="arrangement-canvas"]') as HTMLElement | null
          const r = canvas?.getBoundingClientRect()
          const h = useTransportStore.getState().trackHeight
          if (canvas && r) {
            const at = { bubbles: true, clientX: r.left + 230, clientY: r.top + 22 + 18 * h + h / 2 }
            canvas.dispatchEvent(new MouseEvent('mousemove', at))
          }
        }
      }, 900)
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="RyZe Take U TIme">
            <HwSecondRow />
          </HwTopbar>
          <div className="fl-playlist" style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <HwPlaylistTools />
            <div className="fl-pl-body" style={{ ['--row-h' as never]: '28px' }}>
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
    case 'samplerwindow': {
      // The sample window on a sampler channel: the first channel without
      // audio clips gets the built-in sampler as its instrument. The IPC
      // mock keys its parameter table on the slot id.
      const st = useTrackStore.getState()
      const track = st.tracks.find((t) => t.kind !== 'Master' && !t.clips.some((c) => c.kind.toLowerCase() === 'audio'))
      if (track && !useSampleWindowStore.getState().target) {
        const slot = { id: 'hardwave.native.sampler', pluginId: 'hardwave.native.sampler', pluginName: 'Hardwave Sampler', enabled: true, wet: 1, sidechainSource: null }
        useTrackStore.setState({
          tracks: st.tracks.map((t) => t.id !== track.id ? t : { ...t, name: 'Industrial Kick', color: '#d4a72c', inserts: [slot, ...(t.inserts ?? [])] }),
        })
        useSampleWindowStore.getState().openSampler(track.id, slot.id)
        ;(window as unknown as { __hwMockTracks?: unknown }).__hwMockTracks = useTrackStore.getState().tracks
      }
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <SampleWindow />
        </div>
      )
    }
    case 'tunenotes': {
      // The sample window's NOTES view on a vocal take, as the mockup draws
      // it; scripts/screenshot.mjs opens the view and drags a note.
      const st = useTrackStore.getState()
      const track = st.tracks.find((t) => t.clips.some((c) => c.kind.toLowerCase() === 'audio'))
      const clip = track?.clips.find((c) => c.kind.toLowerCase() === 'audio')
      if (track && clip && !useSampleWindowStore.getState().target) {
        useTransportStore.setState({ bpm: 150 })
        useTrackStore.setState({
          tracks: st.tracks.map((t) => t.id !== track.id ? t : {
            ...t,
            name: 'Vocal',
            clips: t.clips.map((c) => c.id !== clip.id ? c : {
              ...c,
              name: 'Vocal chop 150',
              sourceFrames: Math.round(4.8 * 48000),
              sourceRate: 48000,
              sourceFile: 'C:\\Vocals\\Vocal chop 150.wav',
            }),
          }),
        })
        useSampleWindowStore.getState().open(track.id, clip.id)
        ;(window as unknown as { __hwMockTracks?: unknown }).__hwMockTracks = useTrackStore.getState().tracks
      }
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <SampleWindow />
        </div>
      )
    }
    case 'samplewindow': {
      // The sample window on the playlist's first audio clip, set up as an
      // imported FL sample would be: quieter, a fade out, its DC taken off.
      const st = useTrackStore.getState()
      const track = st.tracks.find((t) => t.clips.some((c) => c.kind.toLowerCase() === 'audio'))
      const clip = track?.clips.find((c) => c.kind.toLowerCase() === 'audio')
      if (track && clip && !useSampleWindowStore.getState().target) {
        useTrackStore.setState({
          tracks: st.tracks.map((t) => t.id !== track.id ? t : {
            ...t,
            clips: t.clips.map((c) => c.id !== clip.id ? c : {
              ...c,
              gainDb: -4,
              fadeOutMs: 375,
              sourceFrames: Math.round(3.2 * 48000),
              sourceRate: 48000,
              sourceFile: 'C:\\Samples\\Crash #1.wav',
              treatment: { pan: 0, polarityInverted: false, swapStereo: false, normalize: false, removeDc: true, resample: false, fadeStereo: false, start: 0, length: 1, trim: 0, declick: 'Out only', loadRegions: false, loadSlices: false },
            }),
          }),
        })
        useSampleWindowStore.getState().open(track.id, clip.id)
        // A change reloads the session; give the mock this one to hand back.
        ;(window as unknown as { __hwMockTracks?: unknown }).__hwMockTracks = useTrackStore.getState().tracks
      }
      return (
        <div className="fl-app" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: '#08080c' }}>
          <HwTopbar showPlaylist showChannelRack={false} showPianoRoll={false} showMixer={false} projectName="Untitled">
            <HwSecondRow />
          </HwTopbar>
          <Arrangement onSetHint={noop} />
          <SampleWindow />
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
