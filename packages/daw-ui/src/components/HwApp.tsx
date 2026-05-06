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

import { useState, useCallback, useMemo } from 'react'
import { Browser } from './browser/Browser'
import { Arrangement } from './arrangement/Arrangement'
import { ChannelRack } from './channelrack/ChannelRack'
import { PianoRoll } from './piano-roll/PianoRoll'
import { MixerPanel } from './mixer/MixerPanel'
import { HwTopMenu, type MenuDef } from './HwTopMenu'
import { useTransportStore } from '../stores/transportStore'
import { useTrackStore } from '../stores/trackStore'
import { usePatternStore } from '../stores/patternStore'
import { usePanelLayoutStore } from '../stores/panelLayoutStore'
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
        <div className={`fl-trans-btn ${recording ? 'rec' : ''}`} title="Record">
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
  const patterns = usePatternStore(s => s.patterns)
  const activeId = usePatternStore(s => s.activeId)
  const setActive = usePatternStore(s => s.setActive)
  const tracks = useTrackStore(s => s.tracks)
  const [tab, setTab] = useState<PickerTab>('ALL')

  const audioTracks = useMemo(() => tracks.filter(t => t.kind === 'Audio'), [tracks])
  const automationTracks = useMemo(() => tracks.filter(t => t.kind === 'Automation'), [tracks])

  const showPatterns = tab === 'ALL' || tab === 'PAT'
  const showAudio = tab === 'ALL' || tab === 'AUD'
  const showAuto = tab === 'ALL' || tab === 'AUT'

  const totalCount =
    (showPatterns ? patterns.length : 0) +
    (showAudio ? audioTracks.length : 0) +
    (showAuto ? automationTracks.length : 0)

  return (
    <div className="fl-picker">
      <div className="fl-picker-head">
        PICKER<span className="ct">{totalCount}</span>
      </div>
      <div className="fl-picker-tabs">
        {(['ALL', 'PAT', 'AUD', 'AUT'] as PickerTab[]).map(t => (
          <button key={t} type="button" className={`fl-picker-tab${tab === t ? ' on' : ''}`} onClick={() => setTab(t)}>
            {t}
          </button>
        ))}
      </div>
      <div className="fl-picker-list">
        {showPatterns && patterns.map(p => (
          <div
            key={p.id}
            className={`fl-pi${p.id === activeId ? ' on' : ''}`}
            style={{ ['--col' as any]: p.color || '#22c55e' }}
            onClick={() => setActive(p.id)}
            title={`Switch to ${p.name}`}
          >
            <span className="ic" />
            <span className="nm">▸ {p.name}</span>
          </div>
        ))}
        {showAudio && audioTracks.map(t => (
          <div
            key={`aud-${t.id}`}
            className="fl-pi"
            style={{ ['--col' as any]: '#06b6d4' }}
            title={t.name}
          >
            <span className="ic" />
            <span className="nm">♫ {t.name}</span>
          </div>
        ))}
        {showAuto && automationTracks.map(t => (
          <div
            key={`aut-${t.id}`}
            className="fl-pi"
            title={t.name}
          >
            <span className="ic" style={{ background: '#1a0e26', border: '1px solid var(--purple)' }} />
            <span className="nm" style={{ color: 'var(--purple)' }}>⌇ {t.name}</span>
          </div>
        ))}
        {totalCount === 0 && (
          <div style={{ padding: '10px 8px', color: 'var(--text-dim)', fontSize: 9, fontFamily: 'var(--mono)' }}>
            No items
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
  const tracks = useTrackStore(s => s.tracks)
  const placeholderCount = Math.max(0, PLAYLIST_TOTAL_SLOTS - tracks.length)
  return (
    <div className="fl-pl-tracks">
      <div className="fl-pl-tracks-head">TRACKS</div>
      <div className="fl-pl-tracks-list">
        {tracks.map((t, i) => (
          <div
            key={t.id}
            className="fl-tr"
            style={{ ['--track-color' as any]: t.color || '#06b6d4' }}
            title={t.name}
          >
            <span className="num">{i + 1}</span>
            <span className="led off"></span>
            <span className="nm">{t.name}</span>
          </div>
        ))}
        {Array.from({ length: placeholderCount }, (_, i) => {
          const slotNum = tracks.length + i + 1
          return (
            <div
              key={`pl-slot-${slotNum}`}
              className="fl-tr fl-tr-empty"
              style={{ ['--track-color' as any]: 'transparent' }}
              aria-hidden="true"
            >
              <span className="num">{slotNum}</span>
              <span className="led off"></span>
              <span className="nm" />
            </div>
          )
        })}
      </div>
    </div>
  )
}

// ─── Playlist HTML ruler (mockup: .fl-pl-ruler) ─────────────────────────────

function HwPlaylistRuler({ totalBars = 64, step = 4 }: { totalBars?: number; step?: number }) {
  // Render a bar-marker every `step` bars, evenly spaced via flex.
  // Width scales with --h-zoom on the parent (.fl-pl-body) per mockup.
  const markers: number[] = []
  for (let bar = 1; bar <= totalBars; bar += step) markers.push(bar)
  return (
    <div className="fl-pl-ruler">
      {markers.map(b => (
        <i key={b}>{b}</i>
      ))}
    </div>
  )
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
  const projectName = '[ddeboer] · Untitled.flp'
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
