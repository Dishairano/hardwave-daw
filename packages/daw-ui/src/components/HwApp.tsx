/*
 * HwApp.tsx — wholesale port of the Hardwave DAW redesign mockup.
 *
 * Renders the full FL-Studio-flavored shell from `daw-mockup/index.html`
 * (.fl-app / .fl-topbar / .fl-second-row / .fl-body / .fl-playlist /
 * .fl-picker / .fl-browser / .fl-cr / .fl-pr / .fl-mx) and wires it to
 * the existing dev stores (transportStore, trackStore, patternStore).
 *
 * The heavy components (Browser, Arrangement, ChannelRack, PianoRoll,
 * MixerPanel) are reused as-is — HwApp only provides the chrome.
 *
 * Replaces `MainLayout` from App.tsx. The CSS lives in `../mockup.css`.
 */

import { useState, useCallback, useEffect, useMemo } from 'react'
import { Browser } from './browser/Browser'
import { Arrangement } from './arrangement/Arrangement'
import { ChannelRack } from './channelrack/ChannelRack'
import { PianoRoll } from './piano-roll/PianoRoll'
import { MixerPanel } from './mixer/MixerPanel'
import { HwTopMenu, type MenuDef } from './HwTopMenu'
import { AutomationLane } from './AutomationLane'
import { KickSynthEditor } from './KickSynthEditor'
import type { AutomationTargetInfo } from '../stores/trackStore'
import { useTransportStore } from '../stores/transportStore'
import { useTrackStore } from '../stores/trackStore'
import { usePatternStore } from '../stores/patternStore'
import { usePickerStore } from '../stores/pickerStore'
import { usePanelLayoutStore } from '../stores/panelLayoutStore'
import { useProjectStore } from '../stores/projectStore'
import type { MobilePanel } from './MobileTabBar'

interface HwAppProps {
  showBrowser: boolean
  showPlaylist: boolean
  showChannelRack: boolean
  showPianoRoll: boolean
  showMixer: boolean
  isMobile: boolean
  mobilePanel: MobilePanel
  /**
   * Top-bar menu structure. Built in App.tsx (where all dialog state +
   * project handlers live) and passed down so HwApp doesn't need to
   * thread 30+ individual callbacks through its prop interface.
   * If omitted, the menu strip falls back to the legacy non-functional
   * label set so existing tests/storybooks don't break.
   */
  menus?: MenuDef[]
  onTogglePianoRoll?: () => void
  onToggleChannelRack?: () => void
  onToggleMixer?: () => void
  onToggleBrowser?: () => void
}

/** Format `positionSamples` as "BAR : BEAT : TICK" using transport store metadata. */
function useTransportClock() {
  const bpm = useTransportStore(s => s.bpm)
  const sampleRate = useTransportStore(s => s.sampleRate)
  const positionSamples = useTransportStore(s => s.positionSamples)
  const tsNum = useTransportStore(s => s.timeSigNumerator)

  const seconds = sampleRate > 0 ? positionSamples / sampleRate : 0
  const beatsPerBar = tsNum > 0 ? tsNum : 4
  const beats = bpm > 0 ? (seconds * bpm / 60) : 0
  const bar = Math.floor(beats / beatsPerBar) + 1
  const beat = Math.floor(beats % beatsPerBar) + 1
  const tick = Math.floor((beats % 1) * 96)
  const min = Math.floor(seconds / 60)
  const sec = seconds % 60

  return {
    barBeatTick: `${bar} : ${beat} : ${String(tick).padStart(2, '0')}`,
    minSec: `${min}:${sec.toFixed(3).padStart(6, '0')}`,
  }
}

// ─── Top bar (fl-topbar) ─────────────────────────────────────────────────────

function HwTopbar({ menus }: { menus?: MenuDef[] }) {
  const playing = useTransportStore(s => s.playing)
  const recording = useTransportStore(s => s.recording)
  const looping = useTransportStore(s => s.looping)
  const bpm = useTransportStore(s => s.bpm)
  const masterDb = useTransportStore(s => s.masterVolumeDb)
  const togglePlayback = useTransportStore(s => s.togglePlayback)
  const stop = useTransportStore(s => s.stop)
  const setPosition = useTransportStore(s => s.setPosition)
  const toggleLoop = useTransportStore(s => s.toggleLoop)
  const toggleRecording = useTransportStore(s => s.toggleRecording)
  const setBpm = useTransportStore(s => s.setBpm)

  const { barBeatTick, minSec } = useTransportClock()

  const patterns = usePatternStore(s => s.patterns)
  const activeId = usePatternStore(s => s.activeId)
  const activePattern = patterns.find(p => p.id === activeId)

  const [editingBpm, setEditingBpm] = useState(false)
  const [bpmDraft, setBpmDraft] = useState('')

  const onWindowMin = useCallback(async () => {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      await getCurrentWindow().minimize()
    } catch {}
  }, [])
  const onWindowMax = useCallback(async () => {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      await getCurrentWindow().toggleMaximize()
    } catch {}
  }, [])
  const onWindowClose = useCallback(async () => {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      await getCurrentWindow().close()
    } catch {}
  }, [])

  const commitBpm = () => {
    const n = parseFloat(bpmDraft)
    if (isFinite(n) && n > 0) setBpm(Math.max(20, Math.min(999, n)))
    setEditingBpm(false)
  }

  return (
    <>
    <div className="fl-topbar">
      <div className="fl-logo">HARD<span>WAVE</span></div>
      {menus && menus.length > 0 ? (
        <HwTopMenu menus={menus} />
      ) : (
        <div className="fl-menu">
          <span>File</span>
          <span>Edit</span>
          <span>Add</span>
          <span>Patterns</span>
          <span>View</span>
          <span>Options</span>
          <span>Tools</span>
          <span>Help</span>
        </div>
      )}
      <div className="fl-topbar-spacer" />
      <div className="fl-win-ctl">
        <i onClick={onWindowMin} title="Minimize">
          <svg className="ic" width="12" height="12" viewBox="0 0 16 16" fill="none">
            <path d="M3 12.5h10" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round"/>
          </svg>
        </i>
        <i onClick={onWindowMax} title="Maximize">
          <svg className="ic" width="11" height="11" viewBox="0 0 16 16" fill="none">
            <rect x="3.5" y="3.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="1.3"/>
          </svg>
        </i>
        <i className="x" onClick={onWindowClose} title="Close">
          <svg className="ic" width="11" height="11" viewBox="0 0 16 16" fill="none">
            <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round"/>
          </svg>
        </i>
      </div>
    </div>

    {/* Row 2: transport + clock + bpm + pattern + perf + master meter,
        left-aligned under HARDWAVE. Window controls stay in row 1. */}
    <div className="fl-toolrow">
      <div className="fl-trans">
        <div className="fl-trans-btn" title="Skip back" onClick={() => setPosition(0)}>
          <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
            <path d="M3.5 2v12M5.5 8l8 5.5V2.5z" fill="currentColor" stroke="currentColor" strokeLinejoin="round" strokeWidth="1.2"/>
          </svg>
        </div>
        <div className="fl-trans-btn" title="Stop" onClick={() => stop()}>
          <svg className="ic" width="11" height="11" viewBox="0 0 16 16" fill="none">
            <rect x="3.5" y="3.5" width="9" height="9" rx="1" fill="currentColor"/>
          </svg>
        </div>
        <div className={`fl-trans-btn ${playing ? 'play' : ''}`} title="Play" onClick={() => togglePlayback()}>
          <svg className="ic" width="12" height="12" viewBox="0 0 16 16" fill="none">
            <path d="M4.5 3l8.5 5-8.5 5z" fill="currentColor"/>
          </svg>
        </div>
        <div className={`fl-trans-btn ${recording ? 'rec' : ''}`} title="Record" onClick={() => toggleRecording()}>
          <svg className="ic" width="12" height="12" viewBox="0 0 16 16" fill="none">
            <circle cx="8" cy="8" r="4.5" fill="currentColor"/>
          </svg>
        </div>
        <div className={`fl-trans-btn ${looping ? 'on' : ''}`} title="Loop" onClick={() => toggleLoop()}>
          <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
            <path d="M2.5 6.5a4 4 0 014-4h4M13.5 9.5a4 4 0 01-4 4h-4M11 .5l2.5 2-2.5 2M5 11.5L2.5 13.5l2.5 2"
              fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round"/>
          </svg>
        </div>
      </div>

      <div className="fl-clock" title="Playhead position">
        <div className="fl-clock-stack">
          <small>BAR · BEAT · TICK</small>
          <b>{barBeatTick}</b>
        </div>
        <div className="fl-clock-stack red">
          <small>MIN : SEC</small>
          <b>{minSec}</b>
        </div>
      </div>

      <div
        className="fl-bpm"
        title="Tempo — click to edit"
        onClick={() => {
          if (!editingBpm) {
            setBpmDraft(bpm.toFixed(3))
            setEditingBpm(true)
          }
        }}
      >
        <small>BPM</small>
        {editingBpm ? (
          <input
            autoFocus
            value={bpmDraft}
            onChange={e => setBpmDraft(e.target.value)}
            onBlur={commitBpm}
            onKeyDown={e => {
              if (e.key === 'Enter') commitBpm()
              if (e.key === 'Escape') setEditingBpm(false)
            }}
          />
        ) : (
          <b>{bpm.toFixed(3)}</b>
        )}
      </div>

      <div className="fl-pat-pill" title="Active pattern">
        <svg className="ic" width="10" height="10" viewBox="0 0 16 16" fill="none">
          <path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"/>
        </svg>
        <span>{activePattern?.name || 'Pattern 1'}</span>
      </div>

      <div className="fl-perf">
        <span>RAM <b>—</b></span>
        <span>CPU <b>—</b></span>
      </div>

      <div className="fl-master-vol">
        <span style={{ textTransform: 'uppercase', fontWeight: 600 }}>MASTER</span>
        <div className="fl-meter-mini"><i /><i /></div>
        <span style={{ fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--text)' }}>
          {masterDb >= 0 ? '+' : ''}{masterDb.toFixed(1)}dB
        </span>
      </div>
    </div>
    </>
  )
}

// ─── Second row: hint + status pills ─────────────────────────────────────────

function HwSecondRow({ hint, projectName }: { hint: string; projectName: string }) {
  const tsNum = useTransportStore(s => s.timeSigNumerator)
  const tsDen = useTransportStore(s => s.timeSigDenominator)
  const recording = useTransportStore(s => s.recording)
  const snapValue = useTransportStore(s => s.snapValue)
  const snapEnabled = useTransportStore(s => s.snapEnabled)

  const defaultHint = `${projectName}  ·  Hover anything for live info`
  return (
    <div className="fl-second-row">
      <span className={`fl-hint${hint ? ' active' : ''}`}>
        {hint || defaultHint}
      </span>
      <div className="fl-tag-pill" title="Recording mode">
        {recording ? 'RECORDING' : 'AUTOMATION'}
      </div>
      <div style={{ flex: 1 }} />
      <div className="fl-step-pill" title="Step length">{snapValue} step</div>
      <div className="fl-tag-pill" title="Snap mode">
        SNAP · {snapEnabled ? snapValue : 'OFF'}
      </div>
      <div className="fl-tag-pill" title="Time signature">{tsNum}/{tsDen}</div>
    </div>
  )
}

// ─── Pattern picker (fl-picker) ─────────────────────────────────────────────

type PickerTab = 'ALL' | 'PAT' | 'AUD' | 'AUT'

function HwPicker() {
  // Picker shows patterns + audio tracks that have content + automation
  // tracks that have lanes. The 500 pre-allocated empty Inserts
  // (v0.158.0) stay out — only tracks the user has actually used join
  // the picker. Tabs filter by category.
  const patterns = usePatternStore(s => s.patterns)
  const activeId = usePatternStore(s => s.activeId)
  const setActive = usePatternStore(s => s.setActive)
  const tracks = useTrackStore(s => s.tracks)
  const pickerSelection = usePickerStore(s => s.selection)
  const togglePattern = usePickerStore(s => s.togglePattern)
  const toggleAudioClip = usePickerStore(s => s.toggleAudioClip)
  const toggleAutomation = usePickerStore(s => s.toggleAutomation)
  const [tab, setTab] = useState<PickerTab>('ALL')

  // The picker is sample-level (FL Studio model): one row per unique
  // sample, regardless of how many times it's been placed in the
  // playlist. Dedup key is the clip name (= file basename without
  // extension), matching how the Channel Rack auto-creates exactly
  // one entry per filename. A source_id-based key drifted out of sync
  // with the Channel Rack: two drops of the same file sometimes
  // produced different source_ids (path normalization, re-imports
  // across save/load) which surfaced as duplicate picker rows even
  // though the Channel Rack showed one entry.
  const audioClipEntries = useMemo(() => {
    const seen = new Set<string>()
    const out: Array<{
      key: string
      name: string
      color: string
      trackId: string
      clipId: string
      trackName: string
    }> = []
    for (const t of tracks) {
      if (t.kind !== 'Audio' || !t.clips || t.clips.length === 0) continue
      for (const c of t.clips) {
        const dedupKey = c.name || t.name
        if (seen.has(dedupKey)) continue
        seen.add(dedupKey)
        out.push({
          key: dedupKey,
          name: dedupKey,
          color: t.color || '#06b6d4',
          trackId: t.id,
          clipId: c.id,
          trackName: t.name,
        })
      }
    }
    return out
  }, [tracks])
  const automationTracksWithLanes = useMemo(
    () => tracks.filter(
      t => t.kind === 'Automation' && t.automationLanes && t.automationLanes.length > 0,
    ),
    [tracks],
  )

  const showPatterns = tab === 'ALL' || tab === 'PAT'
  const showAudio = tab === 'ALL' || tab === 'AUD'
  const showAuto = tab === 'ALL' || tab === 'AUT'

  const totalCount =
    (showPatterns ? patterns.length : 0) +
    (showAudio ? audioClipEntries.length : 0) +
    (showAuto ? automationTracksWithLanes.length : 0)

  return (
    <div className="fl-picker">
      <div className="fl-picker-head">
        PICKER<span className="ct">{totalCount}</span>
      </div>
      <div className="fl-picker-tabs">
        {(['ALL', 'PAT', 'AUD', 'AUT'] as PickerTab[]).map(t => (
          <button
            key={t}
            type="button"
            className={`fl-picker-tab${tab === t ? ' on' : ''}`}
            onClick={() => setTab(t)}
          >
            {t}
          </button>
        ))}
      </div>
      <div className="fl-picker-list">
        {showPatterns && patterns.map(p => {
          const isPickerSelected =
            pickerSelection?.kind === 'pattern' && pickerSelection.patternId === p.id
          return (
            <div
              key={p.id}
              className={`fl-pi${p.id === activeId ? ' on' : ''}${isPickerSelected ? ' picked' : ''}`}
              style={{ ['--col' as any]: p.color || '#22c55e' }}
              onClick={() => {
                // First-click activates for Channel Rack editing AND
                // arms the playlist place-mode; second-click on the
                // same item clears the place-mode but keeps it active
                // in the Channel Rack.
                setActive(p.id)
                togglePattern(p.id)
              }}
              title={`Select ${p.name} — left-click in the playlist to place`}
            >
              <span className="ic" />
              <span className="nm">▸ {p.name}</span>
            </div>
          )
        })}
        {showAudio && audioClipEntries.map(e => {
          const isPickerSelected =
            pickerSelection?.kind === 'audioClip' &&
            pickerSelection.trackId === e.trackId &&
            pickerSelection.clipId === e.clipId
          return (
            <div
              key={e.key}
              className={`fl-pi${isPickerSelected ? ' picked' : ''}`}
              style={{ ['--col' as any]: e.color }}
              onClick={() => toggleAudioClip(e.trackId, e.clipId)}
              title={`${e.name} — left-click in the playlist to place a copy`}
            >
              <span className="ic" />
              <span className="nm">♫ {e.name}</span>
            </div>
          )
        })}
        {showAuto && automationTracksWithLanes.map(t => {
          const firstLane = t.automationLanes[0]
          const isPickerSelected =
            pickerSelection?.kind === 'automation' &&
            pickerSelection.trackId === t.id &&
            pickerSelection.laneId === firstLane?.id
          return (
            <div
              key={`aut-${t.id}`}
              className={`fl-pi${isPickerSelected ? ' picked' : ''}`}
              onClick={() => firstLane && toggleAutomation(t.id, firstLane.id)}
              title={`${t.name} — ${t.automationLanes.length} lane${t.automationLanes.length === 1 ? '' : 's'}`}
            >
              <span className="ic" style={{ background: '#1a0e26', border: '1px solid var(--purple)' }} />
              <span className="nm" style={{ color: 'var(--purple)' }}>⌇ {t.name}</span>
            </div>
          )
        })}
        {totalCount === 0 && (
          <div style={{ padding: '10px 8px', color: 'var(--text-dim)', fontSize: 9, fontFamily: 'var(--mono)' }}>
            {tab === 'ALL' && 'Drop a sample or create a pattern'}
            {tab === 'PAT' && 'No patterns yet'}
            {tab === 'AUD' && 'No audio with content yet'}
            {tab === 'AUT' && 'No automation lanes yet'}
          </div>
        )}
      </div>
    </div>
  )
}

// ─── Playlist tools row ──────────────────────────────────────────────────────

function HwPlaylistTools() {
  const snapValue = useTransportStore(s => s.snapValue)
  const snapEnabled = useTransportStore(s => s.snapEnabled)
  const toggleSnap = useTransportStore(s => s.toggleSnap)
  const tsNum = useTransportStore(s => s.timeSigNumerator)
  const tsDen = useTransportStore(s => s.timeSigDenominator)
  const horizontalZoom = useTransportStore(s => s.horizontalZoom)

  return (
    <div className="fl-pl-tools">
      <button type="button" className="fl-tool on" title="Select tool (S)">
        <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
          <path d="M3 2.5l8.5 4.5-4.2 1.6L5.5 13z" fill="currentColor" stroke="currentColor" strokeWidth=".8" strokeLinejoin="round"/>
        </svg>
      </button>
      <button type="button" className="fl-tool" title="Draw tool (P)">
        <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
          <path d="M2 14l1-3 8-8 2 2-8 8z M10 4l2 2" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round"/>
        </svg>
      </button>
      <button type="button" className="fl-tool" title="Paint tool (B)">
        <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
          <path d="M9 4l3 3-5 5c-1 1-3 1-3 0s1-1.5 1-3z M9 4l3-2 2 2-2 3z" fill="currentColor" stroke="currentColor" strokeWidth=".8" strokeLinejoin="round"/>
        </svg>
      </button>
      <button type="button" className="fl-tool" title="Slice tool (C)">
        <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
          <circle cx="4" cy="11" r="2" fill="none" stroke="currentColor" strokeWidth="1.2"/>
          <circle cx="12" cy="11" r="2" fill="none" stroke="currentColor" strokeWidth="1.2"/>
          <path d="M5.5 9.5L13 2.5M10.5 9.5L3 2.5" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round"/>
        </svg>
      </button>
      <button type="button" className="fl-tool" title="Mute tool (T)">
        <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
          <path d="M3 6.5h2l3-2.5v8L5 9.5H3z" fill="currentColor"/>
          <path d="M11 5l4 6M15 5l-4 6" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round"/>
        </svg>
      </button>
      <button type="button" className="fl-tool" title="Delete tool (D)">
        <svg className="ic" width="12" height="12" viewBox="0 0 16 16" fill="none">
          <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/>
        </svg>
      </button>
      <div className="fl-tool-sep" />
      <button
        type="button"
        className={`fl-tool-pill${snapEnabled ? ' on' : ''}`}
        onClick={() => toggleSnap()}
        title={snapEnabled ? 'Snap ON — click to disable' : 'Snap OFF — click to enable'}
      >
        <small>SNAP</small>{snapValue}
      </button>
      <div className="fl-tool-pill" title="Grid display">
        <small>GRID</small>BARS
      </div>
      <div style={{ flex: 1 }} />
      <div className="fl-tool-pill" title="Time signature">
        <small>SIG</small>{tsNum}/{tsDen}
      </div>
      <div className="fl-tool-pill" title="Horizontal zoom">
        <small>H-ZOOM</small>{Math.round(horizontalZoom * 100)}%
      </div>
    </div>
  )
}

// ─── Playlist track-name column (mockup: .fl-pl-tracks) ─────────────────────
//
// The arrangement grid (Arrangement.tsx) always renders 500 lanes per the
// mockup. This column has to mirror that count or the tracks list and the
// grid drift apart vertically — real tracks at the top, empty grid below
// them. We render `tracks.map()` first, then pad with numbered placeholder
// rows so the totals line up.
const PLAYLIST_TOTAL_SLOTS = 500

function HwPlaylistTracks() {
  const allTracks = useTrackStore(s => s.tracks)
  // Playlist sidebar shows ONLY the pre-allocated inserts. Channels
  // (non-insert tracks created via Channel Rack [+] or audio drop)
  // belong in the Channel Rack panel, not here. Master always stays
  // out — it lives in the mixer.
  const tracks = allTracks.filter(t => t.kind !== 'Master' && t.id.startsWith('insert-'))
  const toggleArm = useTrackStore(s => s.toggleArm)
  const addAutomationLane = useTrackStore(s => s.addAutomationLane)
  const setTrackInstrument = useTrackStore(s => s.setTrackInstrument)
  const placeholderCount = Math.max(0, PLAYLIST_TOTAL_SLOTS - tracks.length)
  // Which track currently has its KickSynth editor open, if any.
  // Editor pops up when the user double-clicks the instrument badge
  // on a kick_synth-voiced track.
  const [kickEditorTrack, setKickEditorTrack] = useState<string | null>(null)
  return (
    <div className="fl-pl-tracks">
      <div className="fl-pl-tracks-head">TRACKS</div>
      <div className="fl-pl-tracks-list">
        {tracks.flatMap((t, i) => {
          const isMidi = (t.kind || '').toLowerCase() === 'midi'
          const row = (
            <div
              key={t.id}
              className={`fl-tr${t.armed ? ' armed' : ''}`}
              style={{ ['--track-color' as any]: t.color || '#06b6d4' }}
              title={t.name}
            >
              <span className="led off"></span>
              <span className="nm">{t.name}</span>
              {isMidi && (
                <HwInstrumentPicker
                  trackId={t.id}
                  current={(t.instrument as any) || 'builtin_sine'}
                  onPick={setTrackInstrument}
                  onOpenEditor={
                    t.instrument === 'kick_synth'
                      ? () => setKickEditorTrack(t.id)
                      : undefined
                  }
                />
              )}
              <button
                type="button"
                className={`fl-tr-arm${t.armed ? ' on' : ''}`}
                onClick={(e) => { e.stopPropagation(); toggleArm(t.id) }}
                title={t.armed ? 'Track armed — click to disarm' : 'Arm for recording'}
                aria-label={t.armed ? `Disarm ${t.name}` : `Arm ${t.name} for recording`}
              >
                R
              </button>
            </div>
          )
          // Render the track's automation lanes directly under it.
          // Phase 1 ships volume + pan; future commits expose plugin-
          // param targets through the (currently hard-coded volume)
          // add-lane button.
          const laneRows = t.automationLanes.map(lane => (
            <AutomationLane key={lane.id} trackId={t.id} lane={lane} />
          ))
          const addLane = (
            <HwAddLaneButton
              key={`addlane-${t.id}`}
              trackId={t.id}
              onAdd={addAutomationLane}
            />
          )
          return [row, ...laneRows, addLane]
        })}
        {Array.from({ length: placeholderCount }, (_, i) => {
          const slotNum = tracks.length + i + 1
          return (
            <div
              key={`pl-slot-${slotNum}`}
              className="fl-tr fl-tr-empty"
              style={{ ['--track-color' as any]: 'transparent' }}
              aria-hidden="true"
            >
              <span className="led off"></span>
              <span className="nm" />
            </div>
          )
        })}
      </div>
      {kickEditorTrack && (() => {
        const t = tracks.find(x => x.id === kickEditorTrack)
        if (!t) return null
        return (
          <KickSynthEditor
            trackId={t.id}
            patchLayers={t.kickPatch?.layers ?? [null, null, null, null]}
            drive={t.kickPatch?.drive ?? 0}
            onClose={() => setKickEditorTrack(null)}
          />
        )
      })()}
    </div>
  )
}

// ─── Add automation lane button ─────────────────────────────────────────────
// Click → opens a small popover to pick the target. Self-contained so we
// don't have to thread Volume/Pan picker state through HwPlaylistTracks.

const LANE_TARGETS: { spec: AutomationTargetInfo; label: string }[] = [
  { spec: { kind: 'track_volume' }, label: 'Volume' },
  { spec: { kind: 'track_pan' },    label: 'Pan' },
  { spec: { kind: 'track_mute' },   label: 'Mute' },
]

function HwAddLaneButton({
  trackId,
  onAdd,
}: {
  trackId: string
  onAdd: (trackId: string, target: AutomationTargetInfo) => Promise<string>
}) {
  const [open, setOpen] = useState(false)
  useEffect(() => {
    if (!open) return
    const close = () => setOpen(false)
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [open])
  return (
    <div style={{ position: 'relative', gridColumn: '1 / -1' }}>
      <button
        type="button"
        className="fl-add-lane"
        onMouseDown={(e) => { e.stopPropagation(); setOpen(v => !v) }}
        title="Add an automation lane — pick its target"
      >
        + Automation lane
      </button>
      {open && (
        <div
          className="fl-lane-target-pop"
          onMouseDown={(e) => e.stopPropagation()}
        >
          {LANE_TARGETS.map(t => (
            <div
              key={t.label}
              className="item"
              onClick={async () => {
                setOpen(false)
                await onAdd(trackId, t.spec)
              }}
            >
              {t.label}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ─── Instrument picker on a MIDI track row ──────────────────────────────────
// Tiny inline dropdown that lets the user swap a MIDI track between the
// default sine monosynth and Hardwave's KickSynth. New voicings land here
// as we ship them.

const NATIVE_INSTRUMENTS: { id: import('../stores/trackStore').NativeInstrumentId; label: string; abbr: string }[] = [
  { id: 'builtin_sine', label: 'Sine (default)', abbr: 'SIN' },
  { id: 'kick_synth',   label: 'KickSynth',      abbr: 'KIK' },
]

function HwInstrumentPicker({
  trackId,
  current,
  onPick,
  onOpenEditor,
}: {
  trackId: string
  current: import('../stores/trackStore').NativeInstrumentId
  onPick: (trackId: string, kind: import('../stores/trackStore').NativeInstrumentId) => Promise<void>
  /** When set, double-clicking the badge opens the instrument's
   *  per-track editor panel. Only wired for kick_synth right now. */
  onOpenEditor?: () => void
}) {
  const [open, setOpen] = useState(false)
  useEffect(() => {
    if (!open) return
    const close = () => setOpen(false)
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [open])
  const currentLabel = NATIVE_INSTRUMENTS.find(n => n.id === current)?.abbr ?? 'SIN'
  return (
    <div style={{ position: 'relative' }}>
      <button
        type="button"
        className={`fl-tr-instr${current === 'kick_synth' ? ' kicksynth' : ''}`}
        onMouseDown={(e) => { e.stopPropagation(); setOpen(v => !v) }}
        onDoubleClick={(e) => { e.stopPropagation(); setOpen(false); onOpenEditor?.() }}
        title={
          onOpenEditor
            ? `${currentLabel} — click to switch · double-click to edit patch`
            : `Native instrument — currently ${currentLabel}`
        }
      >
        {currentLabel}
      </button>
      {open && (
        <div className="fl-tr-instr-pop" onMouseDown={(e) => e.stopPropagation()}>
          {NATIVE_INSTRUMENTS.map(n => (
            <div
              key={n.id}
              className={`item${n.id === current ? ' active' : ''}`}
              onClick={async () => {
                setOpen(false)
                await onPick(trackId, n.id)
              }}
            >
              <span>{n.label}</span>
              {n.id === current && <span className="check">✓</span>}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ─── Playlist HTML ruler (mockup: .fl-pl-ruler) ─────────────────────────────

function HwPlaylistRuler(_props: { totalBars?: number; step?: number }) {
  // Disabled — canvas owns the ruler now (Arrangement.tsx, RULER_HEIGHT
  // = 22). Returning null instead of removing the call site keeps
  // existing layout slots intact in case we want to A/B between
  // canvas + HTML rulers later.
  return null
}

// ─── Playlist panel header ───────────────────────────────────────────────────

function HwPlaylistHead() {
  const patterns = usePatternStore(s => s.patterns)
  const activeId = usePatternStore(s => s.activeId)
  const activePattern = patterns.find(p => p.id === activeId)

  return (
    <div className="fl-pl-head">
      <span className="title" style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
        <svg className="ic" width="9" height="9" viewBox="0 0 16 16" fill="none">
          <path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"/>
        </svg>
        Playlist · Arrangement
        <span style={{ color: 'var(--text-dim)', margin: '0 4px' }}>›</span>
        <b>{activePattern?.name || 'Pattern 1'}</b>
      </span>
      <span style={{ marginLeft: 'auto', display: 'flex', gap: 4, alignItems: 'center', color: 'var(--text-dim)' }}>
        <svg className="ic" width="11" height="11" viewBox="0 0 16 16" fill="none">
          <path d="M3 12.5h10" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round"/>
        </svg>
        <svg className="ic" width="10" height="10" viewBox="0 0 16 16" fill="none">
          <rect x="3.5" y="3.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="1.3"/>
        </svg>
        <svg className="ic" width="10" height="10" viewBox="0 0 16 16" fill="none">
          <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round"/>
        </svg>
      </span>
    </div>
  )
}

// ─── Generic FL panel header (cr / pr / mx) ─────────────────────────────────

function HwPanelHead({
  className,
  title,
  onClose,
}: {
  className: string
  title: string
  onClose?: () => void
}) {
  return (
    <div className={className}>
      <span className="title" style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
        <svg className="ic" width="9" height="9" viewBox="0 0 16 16" fill="none">
          <path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"/>
        </svg>
        {title}
      </span>
      <span style={{ marginLeft: 'auto', display: 'flex', gap: 4, alignItems: 'center', color: 'var(--text-dim)' }}>
        <svg className="ic" width="11" height="11" viewBox="0 0 16 16" fill="none">
          <path d="M3 12.5h10" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round"/>
        </svg>
        <svg className="ic" width="10" height="10" viewBox="0 0 16 16" fill="none">
          <rect x="3.5" y="3.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="1.3"/>
        </svg>
        <svg
          className="ic"
          width="10"
          height="10"
          viewBox="0 0 16 16"
          fill="none"
          style={{ cursor: onClose ? 'pointer' : 'default' }}
          onClick={onClose}
        >
          <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round"/>
        </svg>
      </span>
    </div>
  )
}

// ─── Main HwApp layout ───────────────────────────────────────────────────────

export function HwApp({
  showBrowser,
  showPlaylist,
  showChannelRack,
  showPianoRoll,
  showMixer,
  isMobile,
  mobilePanel,
  menus,
  onTogglePianoRoll,
  onToggleChannelRack,
  onToggleMixer,
}: HwAppProps) {
  const [hint, setHint] = useState('')
  // Read the live project name from the store so save/load actually
  // affects what's shown in the hint bar. Falls back to the friendly
  // default when nothing has been opened yet.
  const projectFileName = useProjectStore(s => s.projectName)
  const projectDirty = useProjectStore(s => s.dirty)
  const projectName = `${projectDirty ? '*' : ''}${projectFileName}.hwp`
  // Mirror the same string into the OS window title via document.title
  // so the taskbar / dock entry matches what the hint bar shows.
  useEffect(() => {
    document.title = `${projectFileName}${projectDirty ? ' *' : ''} — Hardwave DAW`
  }, [projectFileName, projectDirty])
  // Track lane height — drives both the canvas (Arrangement reads from store)
  // and the HTML track-name column (.fl-tr) via the --row-h CSS variable.
  const trackHeight = useTransportStore(s => s.trackHeight)

  // Floating-panel state — when a panel is detached, App.tsx renders it in
  // a FloatingWindow. We must NOT also render the inline version here, or
  // the user sees the same panel twice (one detached, one inline). Persisted
  // in localStorage by panelLayoutStore so this flag survives restarts.
  const layout = usePanelLayoutStore(s => s.layout)

  // Mobile: single panel, no chrome.
  if (isMobile) {
    return (
      <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 0 }}>
        {mobilePanel === 'browser' && <Browser />}
        {mobilePanel === 'channelRack' && <ChannelRack />}
        {mobilePanel === 'pianoRoll' && <PianoRoll />}
        {mobilePanel === 'playlist' && <Arrangement />}
        {mobilePanel === 'mixer' && <MixerPanel />}
      </div>
    )
  }

  return (
    <div className="fl-app" data-testid="hw-app">
      <HwTopbar menus={menus} />
      <HwSecondRow hint={hint} projectName={projectName} />

      <div className="fl-body">
        {showBrowser && !layout.browser.floating && (
          <div className="fl-browser" data-testid="panel-browser">
            <Browser />
          </div>
        )}

        {showPlaylist && <HwPicker />}

        {/* Center column: stacked docked panels */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, overflow: 'hidden' }}>

          {showChannelRack && !layout.channelRack.floating && (
            <div
              className="fl-cr"
              data-testid="panel-channel-rack"
              style={{
                flex: showPlaylist || showPianoRoll ? undefined : 1,
                height: showPlaylist || showPianoRoll ? '40%' : undefined,
                minHeight: 140,
                borderBottom: '1px solid var(--border)',
              }}
            >
              <HwPanelHead
                className="fl-cr-head"
                title="Channel rack"
                onClose={onToggleChannelRack}
              />
              <div className="fl-panel-host">
                <ChannelRack />
              </div>
            </div>
          )}

          {showPianoRoll && !layout.pianoRoll.floating && (
            <div
              className="fl-pr"
              data-testid="panel-piano-roll"
              style={{
                flex: showPlaylist ? undefined : 1,
                height: showPlaylist ? '45%' : undefined,
                minHeight: 200,
                borderBottom: '1px solid var(--border)',
              }}
            >
              <HwPanelHead
                className="fl-pr-head"
                title="Piano roll"
                onClose={onTogglePianoRoll}
              />
              <div className="fl-panel-host">
                <PianoRoll />
              </div>
            </div>
          )}

          {showPlaylist && !layout.playlist.floating && (
            <div className="fl-playlist" data-testid="panel-playlist">
              <HwPlaylistHead />
              <HwPlaylistTools />
              <div
                className="fl-pl-body"
                onMouseLeave={() => setHint('')}
                style={{ ['--row-h' as any]: `${trackHeight}px` }}
              >
                <HwPlaylistTracks />
                <div className="fl-pl-grid">
                  <HwPlaylistRuler />
                  <div className="fl-pl-canvas">
                    <Arrangement onSetHint={setHint} />
                  </div>
                </div>
              </div>
            </div>
          )}

          {showMixer && !layout.mixer.floating && (
            <div
              className="fl-mx"
              data-testid="panel-mixer"
              style={{
                height: showPlaylist || showChannelRack || showPianoRoll ? 220 : undefined,
                flex: showPlaylist || showChannelRack || showPianoRoll ? undefined : 1,
                borderTop: '1px solid var(--border)',
              }}
            >
              <HwPanelHead
                className="fl-mx-head"
                title="Mixer"
                onClose={onToggleMixer}
              />
              <div className="fl-panel-host">
                <MixerPanel />
              </div>
            </div>
          )}

        </div>
      </div>
    </div>
  )
}
