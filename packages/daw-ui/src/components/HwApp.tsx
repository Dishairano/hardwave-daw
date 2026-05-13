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

import { useState, useCallback, useEffect, useMemo, useRef } from 'react'
import { Browser } from './browser/Browser'
import { Arrangement } from './arrangement/Arrangement'
import { ChannelRack } from './channelrack/ChannelRack'
import { PianoRoll } from './piano-roll/PianoRoll'
import { MixerPanel } from './mixer/MixerPanel'
import { HwTopMenu, type MenuDef } from './HwTopMenu'
import { AutomationLane } from './AutomationLane'
import { KickSynthEditor } from './KickSynthEditor'
import type { AutomationTargetInfo } from '../stores/trackStore'
import { useTransportStore, SNAP_VALUES } from '../stores/transportStore'
import { useTrackStore } from '../stores/trackStore'
import { usePatternStore } from '../stores/patternStore'
import { usePickerStore } from '../stores/pickerStore'
import { usePanelLayoutStore } from '../stores/panelLayoutStore'
import { useProjectStore } from '../stores/projectStore'
import { useMetronomeStore } from '../stores/metronomeStore'
import { usePlaylistToolStore, type PlaylistTool } from '../stores/playlistToolStore'
import { useRecordingPrefsStore } from '../stores/recordingPrefsStore'
import { useTypingKeyboardStore } from '../stores/typingKeyboardStore'
import { usePerfMetersStore, startPerfMeters } from '../stores/perfMetersStore'
import type { ActionId } from '../stores/shortcutsStore'
import { invoke } from '@tauri-apps/api/core'
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
  // Ship 1 toolbar port (FL Tier A) — PAT/SONG right-click toggles
  // Channel-Rack / Playlist visibility. Owned by App.tsx.
  onTogglePlaylist?: () => void
  // Tempo right-click + TAP-right-click both open the standalone
  // Tempo Tapper modal (lives in App.tsx alongside other dialogs).
  onOpenTempoTapper?: () => void
  // Ship 2a — toolbar action icons (Save / Save-as / Cut / Copy /
  // Paste / Duplicate) fire through App.tsx's shortcut-dispatch
  // switch so toolbar and keyboard share one code path.
  onAction?: (id: ActionId) => void
  // Render icon opens the existing Export Audio dialog directly —
  // App.tsx owns the dialog's visibility flag.
  onOpenExport?: () => void
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

// ─── Top bar (fl-topbar) — Ship 1 of the Toolbar FL-parity port ────────────
//
// Mockup approved 2026-05-13. Layout follows
// https://suite.hardwavestudios.com/toolbar-fl-parity-mockup/ section 2.
// Ship 1 wires the high-impact controls that all map to existing
// stores: PAT/SONG mode + right-click swap-window, pattern prev/next,
// punch toggle, metronome toggle (with the existing precount menu
// surfacing through the metronome button itself), BPM with a tempo
// context menu (presets + half/double + open Tapper modal), TAP
// button (left-click = inline rolling-avg, right-click = open Tapper),
// time-signature inline editor, master volume drag-slider.
//
// Deferred to Ship 2/3 per the mockup tier ladder: snap pill /
// zoom / 8-tool picker / save-as flashing / render / cut-copy-paste-
// duplicate / step-edit / wait-for-input / count-in / blend / typing-
// keyboard / multilink / master pitch knob / CPU+MEM+POLY meters +
// graph / MIDI activity LED / mini scope / hint-bar icon-types.

function HwTopbar({ menus, onTogglePlaylist, onToggleChannelRack, onOpenTempoTapper, onAction, onOpenExport }: {
  menus?: MenuDef[]
  onTogglePlaylist?: () => void
  onToggleChannelRack?: () => void
  onOpenTempoTapper?: () => void
  onAction?: (id: ActionId) => void
  onOpenExport?: () => void
}) {
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
  const tapTempo = useTransportStore(s => s.tapTempo)
  const patternMode = useTransportStore(s => s.patternMode)
  const setPatternMode = useTransportStore(s => s.setPatternMode)
  const punchEnabled = useTransportStore(s => s.punchEnabled)
  const togglePunch = useTransportStore(s => s.togglePunch)
  const setMasterVolume = useTransportStore(s => s.setMasterVolume)
  const tsNum = useTransportStore(s => s.timeSigNumerator)
  const tsDen = useTransportStore(s => s.timeSigDenominator)
  const setTimeSignature = useTransportStore(s => s.setTimeSignature)
  // Ship 2a — snap pill / zoom / tool picker
  const snapValue = useTransportStore(s => s.snapValue)
  const snapEnabled = useTransportStore(s => s.snapEnabled)
  const setSnapValue = useTransportStore(s => s.setSnapValue)
  const toggleSnap = useTransportStore(s => s.toggleSnap)
  const horizontalZoom = useTransportStore(s => s.horizontalZoom)
  const setHorizontalZoom = useTransportStore(s => s.setHorizontalZoom)
  const zoomToFit = useTransportStore(s => s.zoomToFit)
  const activeTool = usePlaylistToolStore(s => s.tool)
  const setTool = usePlaylistToolStore(s => s.setTool)
  // Ship 3a — recording prefs toggles
  const stepEditing = useRecordingPrefsStore(s => s.stepEditing)
  const toggleStepEditing = useRecordingPrefsStore(s => s.toggleStepEditing)
  const waitForInput = useRecordingPrefsStore(s => s.waitForInput)
  const toggleWaitForInput = useRecordingPrefsStore(s => s.toggleWaitForInput)
  const blendRecord = useRecordingPrefsStore(s => s.blendRecord)
  const toggleBlendRecord = useRecordingPrefsStore(s => s.toggleBlendRecord)
  const multilinkActive = useRecordingPrefsStore(s => s.multilinkActive)
  const toggleMultilink = useRecordingPrefsStore(s => s.toggleMultilink)
  const typingKbdEnabled = useTypingKeyboardStore(s => s.enabled)
  const toggleTypingKbd = useTypingKeyboardStore(s => s.toggle)
  const precountBars = useMetronomeStore(s => s.precountBars)
  const setPrecountBars = useMetronomeStore(s => s.setPrecountBars)

  const { barBeatTick, minSec } = useTransportClock()

  const patterns = usePatternStore(s => s.patterns)
  const activeId = usePatternStore(s => s.activeId)
  const activePattern = patterns.find(p => p.id === activeId)
  const prevPattern = usePatternStore(s => s.prevPattern)
  const nextPattern = usePatternStore(s => s.nextPattern)

  const metronomeEnabled = useMetronomeStore(s => s.enabled)
  const toggleMetronome = useMetronomeStore(s => s.toggleEnabled)

  const undo = useTrackStore(s => s.undo)
  const redo = useTrackStore(s => s.redo)

  const [editingBpm, setEditingBpm] = useState(false)
  const [bpmDraft, setBpmDraft] = useState('')
  const [tempoMenuOpen, setTempoMenuOpen] = useState(false)

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

  // FL tempo right-click presets — verbatim from the manual.
  const tempoPresets = [80, 100, 120, 140, 160]

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

    {/* Row 2 — Ship 1 toolbar layout per approved mockup */}
    <div className="fl-toolrow">
      {/* Undo / Redo */}
      <div style={{ display: 'flex', gap: 2 }}>
        <button onClick={() => undo()} className="fl-mini-btn" title="Undo (Ctrl+Z)">
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.1" strokeLinecap="round" strokeLinejoin="round">
            <polyline points="3.5 4.5 1.5 4.5 1.5 2.5"/>
            <path d="M2 6.5a4 4 0 1 0 1.2-2.8L1.5 5.2"/>
          </svg>
        </button>
        <button onClick={() => redo()} className="fl-mini-btn" title="Redo (Ctrl+Y)">
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.1" strokeLinecap="round" strokeLinejoin="round">
            <polyline points="8.5 4.5 10.5 4.5 10.5 2.5"/>
            <path d="M10 6.5a4 4 0 1 1-1.2-2.8L10.5 5.2"/>
          </svg>
        </button>
      </div>

      <span className="fl-toolsep" />

      {/* PAT / SONG mode toggle. Right-click on PAT toggles Channel
          Rack visibility; right-click on SONG toggles Playlist. */}
      <div className="fl-mode-toggle">
        <button
          className={patternMode ? 'active' : ''}
          onClick={() => setPatternMode(true)}
          onContextMenu={(e) => { e.preventDefault(); onToggleChannelRack?.() }}
          title="Pattern mode · right-click toggles Channel Rack"
        >PAT</button>
        <button
          className={!patternMode ? 'active' : ''}
          onClick={() => setPatternMode(false)}
          onContextMenu={(e) => { e.preventDefault(); onTogglePlaylist?.() }}
          title="Song mode · right-click toggles Playlist"
        >SONG</button>
      </div>

      <span className="fl-toolsep" />

      {/* Pattern pill with prev/next arrows */}
      <div className="fl-pat-pill" title="Active pattern · click arrows to nav">
        <span className="nav" onClick={() => prevPattern()} title="Previous pattern">‹</span>
        <span className="name">{activePattern?.name || 'Pattern 1'}</span>
        <span className="nav" onClick={() => nextPattern()} title="Next pattern">›</span>
      </div>

      <span className="fl-toolsep" />

      {/* Transport cluster: REC · STOP · PLAY · LOOP · PUNCH · METR */}
      <div className="fl-trans">
        <div className={`fl-trans-btn ${recording ? 'rec' : ''}`} title="Record (R) · double-click cancels in-flight take" onClick={() => toggleRecording()}>
          <svg className="ic" width="12" height="12" viewBox="0 0 16 16" fill="none">
            <circle cx="8" cy="8" r="4.5" fill="currentColor"/>
          </svg>
        </div>
        <div className="fl-trans-btn" title="Stop · double-click = panic (stop all sound)" onClick={() => stop()} onDoubleClick={() => { stop(); setPosition(0) }}>
          <svg className="ic" width="11" height="11" viewBox="0 0 16 16" fill="none">
            <rect x="3.5" y="3.5" width="9" height="9" rx="1" fill="currentColor"/>
          </svg>
        </div>
        <div className={`fl-trans-btn ${playing ? 'play' : ''}`} title="Play (Space)" onClick={() => togglePlayback()}>
          <svg className="ic" width="12" height="12" viewBox="0 0 16 16" fill="none">
            <path d="M4.5 3l8.5 5-8.5 5z" fill="currentColor"/>
          </svg>
        </div>
        <div className={`fl-trans-btn ${looping ? 'on' : ''}`} title="Loop (L)" onClick={() => toggleLoop()}>
          <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
            <path d="M2.5 6.5a4 4 0 014-4h4M13.5 9.5a4 4 0 01-4 4h-4M11 .5l2.5 2-2.5 2M5 11.5L2.5 13.5l2.5 2"
              fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round"/>
          </svg>
        </div>
        <div className={`fl-trans-btn ${punchEnabled ? 'punch' : ''}`} title="Punch range" onClick={() => togglePunch()}>
          <svg className="ic" width="12" height="10" viewBox="0 0 12 10" fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M3 1.5H1.5V8.5H3"/>
            <path d="M9 1.5h1.5V8.5H9"/>
          </svg>
        </div>
        <div className={`fl-trans-btn ${metronomeEnabled ? 'metr' : ''}`} title="Metronome (Ctrl+M)" onClick={() => toggleMetronome()}>
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.1" strokeLinejoin="round">
            <path d="M3.5 10.5L5 1.5h2l1.5 9z"/>
            <line x1="2.5" y1="10.5" x2="9.5" y2="10.5"/>
            <line x1="6" y1="6" x2="9.5" y2="3"/>
          </svg>
        </div>
      </div>

      <span className="fl-toolsep" />

      {/* BPM — click to edit, drag to nudge, right-click for menu */}
      <div style={{ position: 'relative' }}>
        <div
          className="fl-bpm"
          title="Tempo · click to edit · drag vertical · right-click for menu"
          onClick={() => {
            if (!editingBpm) {
              setBpmDraft(bpm.toFixed(3))
              setEditingBpm(true)
            }
          }}
          onContextMenu={(e) => { e.preventDefault(); setTempoMenuOpen(v => !v) }}
          onPointerDown={(e) => {
            if (editingBpm) return
            if (e.button !== 0) return
            if (e.target instanceof HTMLInputElement) return
            const startY = e.clientY
            const startBpm = bpm
            const target = e.currentTarget
            target.setPointerCapture(e.pointerId)
            let moved = false
            const onMove = (ev: PointerEvent) => {
              const dy = startY - ev.clientY
              if (Math.abs(dy) > 2) moved = true
              const fine = ev.ctrlKey || ev.metaKey ? 0.1 : 1
              const next = Math.max(20, Math.min(999, startBpm + dy * 0.3 * fine))
              setBpm(Math.round(next * 10) / 10)
            }
            const onUp = (ev: PointerEvent) => {
              target.releasePointerCapture(ev.pointerId)
              target.removeEventListener('pointermove', onMove)
              target.removeEventListener('pointerup', onUp)
              // If the user dragged we suppress the click-to-edit
              // that fires after pointerup — the click is treated as
              // a value-edit, the drag as a value-nudge.
              if (moved) {
                const stop = (ce: MouseEvent) => { ce.stopPropagation(); window.removeEventListener('click', stop, true) }
                window.addEventListener('click', stop, true)
              }
            }
            target.addEventListener('pointermove', onMove)
            target.addEventListener('pointerup', onUp)
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
        {tempoMenuOpen && (
          <HwTempoContextMenu
            bpm={bpm}
            presets={tempoPresets}
            onPreset={(v) => { setBpm(v); setTempoMenuOpen(false) }}
            onHalf={() => { setBpm(Math.max(20, Math.round(bpm * 0.5 * 10) / 10)); setTempoMenuOpen(false) }}
            onDouble={() => { setBpm(Math.min(999, Math.round(bpm * 2 * 10) / 10)); setTempoMenuOpen(false) }}
            onOpenTapper={() => { onOpenTempoTapper?.(); setTempoMenuOpen(false) }}
            onClose={() => setTempoMenuOpen(false)}
          />
        )}
      </div>

      {/* TAP — click = inline tap, right-click = open Tapper modal */}
      <button
        onClick={() => tapTempo()}
        onContextMenu={(e) => { e.preventDefault(); onOpenTempoTapper?.() }}
        className="fl-mini-btn"
        style={{ height: 26, padding: '0 8px', fontSize: 8, fontWeight: 700, letterSpacing: 0.4, fontFamily: 'var(--mono)' }}
        title="Tap tempo · right-click opens Tempo Tapper modal"
      >TAP</button>

      {/* Time signature inline editor */}
      <div className="fl-tsig" title="Time signature">
        <input
          type="number" min={1} max={32} value={tsNum}
          onChange={e => setTimeSignature(Math.max(1, parseInt(e.target.value) || 4), tsDen)}
        />
        <span className="sl">/</span>
        <select value={tsDen} onChange={e => setTimeSignature(tsNum, parseInt(e.target.value))}>
          {[1, 2, 4, 8, 16, 32].map(d => <option key={d} value={d}>{d}</option>)}
        </select>
      </div>

      <span className="fl-toolsep" />

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

      <span className="fl-toolsep" />

      {/* Ship 2a — Snap pill (moved from HwSecondRow). Pill toggles
          on/off via the leading dot; the value-select changes the
          grid resolution and auto-enables snap when set to anything
          other than 'Off' (per setSnapValue's existing semantics). */}
      <div
        className={`fl-snap${snapEnabled ? ' on' : ''}`}
        title={`Snap: ${snapEnabled ? snapValue : 'Off'} · alt-drag bypasses while moving clips`}
      >
        <button onClick={() => toggleSnap()} className="dot" title="Toggle snap" />
        <span className="label">Snap</span>
        <select
          value={snapValue}
          onChange={e => setSnapValue(e.target.value as any)}
          data-testid="hw-toolbar-snap-select"
        >
          {SNAP_VALUES.map(v => <option key={v} value={v}>{v}</option>)}
        </select>
      </div>

      <span className="fl-toolsep" />

      {/* Zoom trio — − / FIT / + */}
      <div className="fl-zoom" title={`Zoom: ${horizontalZoom.toFixed(2)}×`}>
        <button onClick={() => setHorizontalZoom(horizontalZoom / 1.25)} title="Zoom out">−</button>
        <button onClick={() => zoomToFit()} className="fit" title="Zoom to fit">FIT</button>
        <button onClick={() => setHorizontalZoom(horizontalZoom * 1.25)} title="Zoom in">+</button>
      </div>

      <span className="fl-toolsep" />

      {/* 8-tool picker — draw / paint / slice / delete / mute / slip / select / zoom */}
      <div className="fl-tools" role="toolbar" aria-label="Playlist tools">
        {(['draw','paint','slice','delete','mute','slip','select','zoom'] as PlaylistTool[]).map(t => (
          <ToolPickerBtn key={t} tool={t} active={activeTool === t} onClick={() => setTool(t)} />
        ))}
      </div>

      <span className="fl-toolsep" />

      {/* Action icon row — Save / Save-as / Render / Cut / Copy / Paste / Duplicate.
          Save-as flashes as the FL 5/10/30-minute save reminder
          would; the flash class is wired via the projectDirty store
          flag so the cue only fires when there are unsaved edits. */}
      <div className="fl-action-row">
        <button onClick={() => onAction?.('save')} className="fl-mini-btn" title="Save (Ctrl+S)">
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.1" strokeLinejoin="round">
            <path d="M2 2h8v8H2zM4 2v3h4V2M4 10v-3h4v3"/>
          </svg>
        </button>
        <SaveAsButton onClick={() => onAction?.('saveAs')} />
        <button onClick={() => onOpenExport?.()} className="fl-mini-btn" title="Render audio (Ctrl+R)">
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.1" strokeLinecap="round" strokeLinejoin="round">
            <path d="M6 1v6m-3-3 3 3 3-3M2 9v2h8V9"/>
          </svg>
        </button>
        <button onClick={() => onAction?.('cut')} className="fl-mini-btn" title="Cut (Ctrl+X)">
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.1">
            <circle cx="3" cy="9" r="1.5"/><circle cx="9" cy="9" r="1.5"/>
            <path d="M4.5 7.5L11 1M7.5 7.5L1 1"/>
          </svg>
        </button>
        <button onClick={() => onAction?.('copy')} className="fl-mini-btn" title="Copy (Ctrl+C)">
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1">
            <rect x="3" y="3" width="7" height="7" rx="1"/>
            <path d="M3 3V2a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-1"/>
          </svg>
        </button>
        <button onClick={() => onAction?.('paste')} className="fl-mini-btn" title="Paste at playhead (Ctrl+V)">
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1">
            <rect x="2.5" y="3" width="7" height="8" rx="1"/>
            <rect x="4.5" y="1.5" width="3" height="2" rx="0.5"/>
          </svg>
        </button>
        <button onClick={() => onAction?.('duplicate')} className="fl-mini-btn" title="Duplicate selection (Ctrl+D)">
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1">
            <rect x="1.5" y="3.5" width="5" height="5"/>
            <rect x="5.5" y="3.5" width="5" height="5"/>
          </svg>
        </button>
      </div>

      <span className="fl-toolsep" />

      {/* Ship 3a — recording-prefs toggle cluster.
          UI flips the flags and persists them; backend wiring for
          each behaviour ships in follow-up batches (see store doc-
          comment in `recordingPrefsStore.ts`). */}
      <div className="fl-action-row">
        <button onClick={() => toggleStepEditing()} className={`fl-mini-btn${stepEditing ? ' on' : ''}`} title="Step editing (Ctrl+E)">
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1">
            <rect x="1" y="3" width="2" height="6" fill="currentColor"/>
            <rect x="4" y="5" width="2" height="4"/>
            <rect x="7" y="3" width="2" height="6" fill="currentColor"/>
            <rect x="10" y="5" width="2" height="4"/>
          </svg>
        </button>
        <button onClick={() => toggleWaitForInput()} className={`fl-mini-btn${waitForInput ? ' on' : ''}`} title="Wait for input (Ctrl+I)">
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1">
            <circle cx="6" cy="6" r="4"/>
            <path d="M6 4v2.5L8 8" strokeLinecap="round"/>
          </svg>
        </button>
        <button
          onClick={() => setPrecountBars(precountBars === 0 ? 2 : 0)}
          onContextMenu={(e) => {
            e.preventDefault()
            // Cycle 0→1→2→4→0 on right-click for quick bar selection.
            const cycle = [0, 1, 2, 4]
            const idx = cycle.indexOf(precountBars)
            setPrecountBars(cycle[(idx + 1) % cycle.length])
          }}
          className={`fl-mini-btn${precountBars > 0 ? ' on' : ''}`}
          title={`Count-in (Ctrl+P) · ${precountBars === 0 ? 'off' : `${precountBars} bar${precountBars === 1 ? '' : 's'}`} · right-click cycles bars`}
        >
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1">
            <text x="6" y="9" textAnchor="middle" fontSize="8" fontWeight="700" fill="currentColor" fontFamily="JetBrains Mono">
              {precountBars > 0 ? precountBars : '∅'}
            </text>
          </svg>
        </button>
        <button onClick={() => toggleBlendRecord()} className={`fl-mini-btn${blendRecord ? ' on' : ''}`} title="Blend / overdub (Ctrl+B)">
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1">
            <circle cx="5" cy="6" r="3"/>
            <circle cx="8" cy="6" r="3"/>
          </svg>
        </button>
        <button onClick={() => toggleTypingKbd()} className={`fl-mini-btn${typingKbdEnabled ? ' on' : ''}`} title="Typing keyboard → piano (Ctrl+T)">
          <svg className="ic" width="13" height="9" viewBox="0 0 13 9" fill="none" stroke="currentColor" strokeWidth="0.8">
            <rect x="0.5" y="0.5" width="12" height="8" rx="1"/>
            <rect x="2" y="2" width="2" height="2" rx="0.3" fill="currentColor"/>
            <rect x="5.5" y="2" width="2" height="2" rx="0.3" fill="currentColor"/>
            <rect x="9" y="2" width="2" height="2" rx="0.3" fill="currentColor"/>
            <rect x="3" y="5.5" width="7" height="1.5" rx="0.3" fill="currentColor" opacity="0.6"/>
          </svg>
        </button>
        <button onClick={() => toggleMultilink()} className={`fl-mini-btn${multilinkActive ? ' on' : ''}`} title="Multilink to controllers (Ctrl+J)">
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="0.9">
            <circle cx="3" cy="3" r="1.5"/>
            <circle cx="9" cy="9" r="1.5"/>
            <line x1="4.2" y1="4.2" x2="7.8" y2="7.8"/>
          </svg>
        </button>
      </div>

      <span className="fl-toolsep" />

      {/* Ship 3b — live perf meter cluster (CPU + MEM) and the MIDI
          activity LED. Polyphony is intentionally omitted until the
          engine surfaces a voice-count event; we'd rather show two
          honest readouts than three with one fake. */}
      <HwPerfCluster />
      <HwMidiActivityLed />
      <HwMiniScope />

      <div className="fl-master-vol">
        <span style={{ textTransform: 'uppercase', fontWeight: 600, fontSize: 7, color: 'var(--text-dim)', letterSpacing: 0.6 }}>MASTER</span>
        <HwMasterSlider valueDb={masterDb} onChange={setMasterVolume} />
        <span style={{ fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--text)', minWidth: 36, textAlign: 'right' }}>
          {masterDb >= 0 ? '+' : ''}{masterDb.toFixed(1)}dB
        </span>
      </div>
    </div>
    </>
  )
}

// ─── Master volume slider (FL toolbar parity) ──────────────────────────────
//
// Drag horizontally to set master gain in dB. Range −60..+6 like FL.
// Double-click to reset to 0 dB. The store is driven directly via
// setMasterVolume which already clamps + persists.
const MASTER_MIN_DB = -60
const MASTER_MAX_DB = 6
function HwMasterSlider({ valueDb, onChange }: { valueDb: number; onChange: (db: number) => void }) {
  const pct = Math.max(0, Math.min(1, (valueDb - MASTER_MIN_DB) / (MASTER_MAX_DB - MASTER_MIN_DB)))
  const handleDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    const el = e.currentTarget
    const rect = el.getBoundingClientRect()
    const apply = (clientX: number) => {
      const p = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width))
      onChange(MASTER_MIN_DB + p * (MASTER_MAX_DB - MASTER_MIN_DB))
    }
    apply(e.clientX)
    el.setPointerCapture(e.pointerId)
    const onMove = (ev: PointerEvent) => apply(ev.clientX)
    const onUp = (ev: PointerEvent) => {
      el.releasePointerCapture(ev.pointerId)
      el.removeEventListener('pointermove', onMove)
      el.removeEventListener('pointerup', onUp)
    }
    el.addEventListener('pointermove', onMove)
    el.addEventListener('pointerup', onUp)
  }
  return (
    <div
      onPointerDown={handleDrag}
      onDoubleClick={() => onChange(0)}
      style={{
        position: 'relative', width: 80, height: 8,
        background: 'rgba(255,255,255,0.04)',
        border: '1px solid var(--border)',
        borderRadius: 3, cursor: 'ew-resize',
      }}
    >
      <div style={{
        position: 'absolute', left: 0, top: 0, height: '100%',
        width: `${pct * 100}%`,
        background: 'linear-gradient(90deg,#2a2a30,var(--red-bright))',
        borderRadius: 3,
      }} />
    </div>
  )
}

// ─── Tempo right-click menu ────────────────────────────────────────────────
//
// FL's tempo RMB reference set: type-in-value (handled by the inline
// click-to-edit input), preset BPMs (80/100/120/140/160), Half/Double-
// speed shortcuts, and a Tap sub-menu that surfaces the full Tempo
// Tapper modal. Edit events / Create automation clip are stubs for
// Tier B — they appear disabled so the affordance is visible.

function HwTempoContextMenu({
  bpm, presets, onPreset, onHalf, onDouble, onOpenTapper, onClose,
}: {
  bpm: number
  presets: number[]
  onPreset: (v: number) => void
  onHalf: () => void
  onDouble: () => void
  onOpenTapper: () => void
  onClose: () => void
}) {
  useEffect(() => {
    const handle = () => onClose()
    // Defer so the right-click that opened the menu doesn't close it.
    const id = window.setTimeout(() => window.addEventListener('click', handle), 0)
    return () => { window.clearTimeout(id); window.removeEventListener('click', handle) }
  }, [onClose])
  const item: React.CSSProperties = {
    padding: '6px 10px', fontSize: 11, color: 'var(--text)',
    background: 'transparent', border: 'none', textAlign: 'left',
    cursor: 'pointer', borderRadius: 3, width: '100%',
  }
  const itemDisabled: React.CSSProperties = { ...item, color: 'var(--text-dim)', cursor: 'not-allowed' }
  return (
    <div
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      style={{
        position: 'absolute', top: 36, left: 0, zIndex: 500,
        minWidth: 200,
        background: 'rgba(12,12,18,0.97)',
        border: '1px solid var(--border-strong)',
        borderRadius: 6,
        boxShadow: '0 8px 32px rgba(0,0,0,0.55)',
        backdropFilter: 'blur(8px)',
        padding: 6,
        display: 'flex', flexDirection: 'column', gap: 2,
      }}
    >
      <div style={{
        padding: '4px 10px 6px', fontSize: 8, color: 'var(--text-dim)',
        letterSpacing: 0.6, textTransform: 'uppercase',
        borderBottom: '1px solid var(--border)', marginBottom: 4,
      }}>
        Tempo · {bpm.toFixed(1)} BPM
      </div>
      <button style={itemDisabled} disabled title="Coming in Tier B">Edit events…</button>
      <button style={itemDisabled} disabled title="Coming in Tier B">Create automation clip…</button>
      <div style={{ height: 1, background: 'var(--border)', margin: '4px 6px' }} />
      <div style={{ padding: '2px 10px 4px', fontSize: 8, color: 'var(--text-dim)', letterSpacing: 0.6, textTransform: 'uppercase' }}>
        Presets
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(5,1fr)', gap: 3, padding: '0 4px' }}>
        {presets.map(v => {
          const active = Math.abs(bpm - v) < 0.05
          return (
            <button key={v} onClick={() => onPreset(v)} style={{
              padding: '4px 0', fontSize: 9, fontWeight: 600,
              color: active ? 'var(--red-bright)' : 'var(--text)',
              background: active ? 'rgba(220,38,38,0.18)' : 'rgba(255,255,255,0.03)',
              border: `1px solid ${active ? 'rgba(239,68,68,0.4)' : 'var(--border)'}`,
              borderRadius: 3, cursor: 'pointer', fontFamily: 'var(--mono)',
            }}>{v}</button>
          )
        })}
      </div>
      <div style={{ height: 1, background: 'var(--border)', margin: '6px 6px 4px' }} />
      <button style={item} onClick={onHalf}>Half-speed (÷2)</button>
      <button style={item} onClick={onDouble}>Double-speed (×2)</button>
      <div style={{ height: 1, background: 'var(--border)', margin: '4px 6px' }} />
      <button style={item} onClick={onOpenTapper}>Tap tempo…</button>
    </div>
  )
}

// ─── 8-tool picker button ──────────────────────────────────────────────────
//
// Each toolbar slot renders the FL-style line icon. Active tool is
// highlighted via the .on class; tooltips spell out the keybind so
// the user can learn shortcuts without opening the help panel.

const TOOL_LABEL: Record<PlaylistTool, string> = {
  draw: 'Draw (P)',
  paint: 'Paint (B)',
  slice: 'Slice (C)',
  delete: 'Delete (D)',
  mute: 'Mute (T)',
  slip: 'Slip (S)',
  select: 'Select (E)',
  zoom: 'Zoom (Z)',
}

function ToolPickerBtn({ tool, active, onClick }: { tool: PlaylistTool; active: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={active ? 'on' : ''}
      title={TOOL_LABEL[tool]}
      aria-pressed={active}
    >
      {tool === 'draw' && (
        <svg className="ic" viewBox="0 0 10 10" width="11" height="11" fill="none">
          <path d="M1.5 8.5L2 6L7 1L9 3L4 8Z" stroke="currentColor" strokeWidth="0.8" fill={active ? 'currentColor' : 'none'} opacity={active ? 0.3 : 1} />
          <path d="M7 1L9 3" stroke="currentColor" strokeWidth="1" />
        </svg>
      )}
      {tool === 'paint' && (
        <svg className="ic" viewBox="0 0 10 10" width="11" height="11" fill="none">
          <rect x="1" y="6" width="3" height="3.5" rx="0.5" stroke="currentColor" strokeWidth="0.8" />
          <path d="M2.5 6V2.5C2.5 1.5 3.5 0.5 5 0.5H8C8.5 0.5 9 1 9 1.5V3C9 3.5 8.5 4 8 4H5.5L4 5.5" stroke="currentColor" strokeWidth="0.8" />
        </svg>
      )}
      {tool === 'slice' && (
        <svg className="ic" viewBox="0 0 10 10" width="11" height="11" fill="none">
          <path d="M3 1L7 9" stroke="currentColor" strokeWidth="1" strokeLinecap="round" />
          <circle cx="3" cy="1.5" r="1" stroke="currentColor" strokeWidth="0.6" />
        </svg>
      )}
      {tool === 'delete' && (
        <svg className="ic" viewBox="0 0 10 10" width="11" height="11" fill="none">
          <line x1="2" y1="2" x2="8" y2="8" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
          <line x1="8" y1="2" x2="2" y2="8" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
        </svg>
      )}
      {tool === 'mute' && (
        <svg className="ic" viewBox="0 0 10 10" width="11" height="11" fill="none">
          <rect x="1" y="1" width="8" height="8" rx="1" stroke="currentColor" strokeWidth="0.8" />
          <line x1="1" y1="1" x2="9" y2="9" stroke="currentColor" strokeWidth="0.8" />
        </svg>
      )}
      {tool === 'slip' && (
        <svg className="ic" viewBox="0 0 10 10" width="11" height="11" fill="none">
          <rect x="1" y="3" width="8" height="4" rx="0.5" stroke="currentColor" strokeWidth="0.8" />
          <path d="M4 3V7M6 3V7" stroke="currentColor" strokeWidth="0.6" strokeDasharray="1 1" />
        </svg>
      )}
      {tool === 'select' && (
        <svg className="ic" viewBox="0 0 10 10" width="11" height="11" fill="none">
          <path d="M2 1L2 9L5 6.5L7.5 9L8.5 8L6 5.5L9 5L2 1Z" stroke="currentColor" strokeWidth="0.7" fill={active ? 'currentColor' : 'none'} opacity={active ? 0.3 : 1} />
        </svg>
      )}
      {tool === 'zoom' && (
        <svg className="ic" viewBox="0 0 10 10" width="11" height="11" fill="none">
          <circle cx="4.5" cy="4.5" r="3" stroke="currentColor" strokeWidth="0.9" />
          <line x1="7" y1="7" x2="9.5" y2="9.5" stroke="currentColor" strokeWidth="1" strokeLinecap="round" />
          <line x1="3" y1="4.5" x2="6" y2="4.5" stroke="currentColor" strokeWidth="0.7" />
          <line x1="4.5" y1="3" x2="4.5" y2="6" stroke="currentColor" strokeWidth="0.7" />
        </svg>
      )}
    </button>
  )
}

// ─── Save-As flashing reminder button ──────────────────────────────────────
//
// FL Studio flashes the Save button every 5 minutes after the first
// unsaved edit, then every 30 s once you cross 10 minutes. We pick
// up the same cadence from the projectDirty store flag + a local
// elapsed-since-last-save clock. Clicking surfaces the Save-As
// dialog rather than overwriting, matching the FL semantic.

// ─── Live performance meter cluster ────────────────────────────────────────
//
// Subscribes to perfMetersStore (frame-time + heap), renders CPU + MEM
// horizontal bars next to numeric readouts. The store is fed by the
// rAF-driven sampler in `startPerfMeters` — bootstrapped from the
// HwApp body so it lives exactly as long as the desktop app.

function HwPerfCluster() {
  const cpuPct = usePerfMetersStore(s => s.cpuPct)
  const memMb = usePerfMetersStore(s => s.memMb)
  const memRatio = usePerfMetersStore(s => s.memRatio)
  const cpuColor = cpuPct > 80 ? 'var(--red-bright)' : cpuPct > 50 ? 'var(--amber)' : 'var(--green)'
  const memColor = (memRatio ?? 0) > 0.8 ? 'var(--red-bright)' : (memRatio ?? 0) > 0.5 ? 'var(--amber)' : 'var(--cyan)'
  return (
    <div className="fl-perf" title={`CPU ${cpuPct}% (frame-time estimate) · MEM ${memMb ?? '—'} MB`}>
      <span className="fl-perf-stack">
        <small>CPU</small>
        <span className="fl-perf-bar"><i style={{ width: `${cpuPct}%`, background: cpuColor }} /></span>
      </span>
      <span className="fl-perf-stack">
        <small>MEM</small>
        <span className="fl-perf-bar"><i style={{ width: `${(memRatio ?? 0) * 100}%`, background: memColor }} /></span>
      </span>
    </div>
  )
}

// ─── MIDI activity LED ─────────────────────────────────────────────────────
//
// Polls the Rust `get_midi_activity` command at 5 Hz; the LED pulses
// green for ~250 ms each time `ms_since_last_event` ticks below the
// freshness threshold. Falls dark when no port is open. We use a
// short tooltip so users can see the open port name without opening
// MIDI settings.

interface MidiActivitySnapshot {
  open_ports: string[]
  ms_since_last_event: number | null
}

function HwMidiActivityLed() {
  const [snap, setSnap] = useState<MidiActivitySnapshot | null>(null)
  useEffect(() => {
    let cancelled = false
    const poll = async () => {
      try {
        const v = await invoke<MidiActivitySnapshot>('get_midi_activity')
        if (!cancelled) setSnap(v)
      } catch { /* command not registered yet during dev */ }
    }
    poll()
    const id = window.setInterval(poll, 200)
    return () => { cancelled = true; window.clearInterval(id) }
  }, [])
  const active = snap?.ms_since_last_event != null && snap.ms_since_last_event < 250
  const hasPort = (snap?.open_ports.length ?? 0) > 0
  const tooltip = hasPort
    ? `MIDI · ${snap!.open_ports.length} port${snap!.open_ports.length === 1 ? '' : 's'} open · ${snap!.ms_since_last_event ?? '—'} ms since last event`
    : 'MIDI · no input port open'
  return (
    <div className="fl-midi-led" title={tooltip}>
      <span className={`dot${active ? ' active' : ''}${hasPort ? '' : ' dark'}`} />
      <span className="label">MIDI</span>
    </div>
  )
}

// ─── Mini output scope ─────────────────────────────────────────────────────
//
// Placeholder rolling waveform. Real audio-tap wiring (sample buffer
// from the engine's master bus) ships in a follow-up — for now we
// draw a rAF-driven sine that responds to transport playback state
// so the meter feels alive when the user hits Play.

function HwMiniScope() {
  const playing = useTransportStore(s => s.playing)
  const svgRef = useRef<SVGPolylineElement | null>(null)
  useEffect(() => {
    let rafId = 0
    let phase = 0
    const W = 58
    const H = 14
    const tick = () => {
      const el = svgRef.current
      if (el) {
        const pts: string[] = []
        const amplitude = playing ? 5.5 : 1.2
        for (let x = 0; x <= W; x += 2) {
          const y = H / 2 + Math.sin((x / 8) + phase) * amplitude * (0.6 + 0.4 * Math.random())
          pts.push(`${x},${y.toFixed(2)}`)
        }
        el.setAttribute('points', pts.join(' '))
        phase += playing ? 0.3 : 0.06
      }
      rafId = requestAnimationFrame(tick)
    }
    rafId = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(rafId)
  }, [playing])
  return (
    <div className="fl-mini-scope" title="Master output (placeholder — engine tap to follow)">
      <svg width="58" height="14" viewBox="0 0 58 14" preserveAspectRatio="none">
        <polyline ref={svgRef} points="" fill="none" stroke="var(--red-bright)" strokeWidth="0.9" opacity="0.75" />
      </svg>
    </div>
  )
}

function SaveAsButton({ onClick }: { onClick: () => void }) {
  const dirty = useProjectStore(s => s.dirty)
  const [elapsed, setElapsed] = useState(0)
  useEffect(() => {
    if (!dirty) { setElapsed(0); return }
    const start = Date.now()
    const id = window.setInterval(() => setElapsed(Date.now() - start), 1000)
    return () => window.clearInterval(id)
  }, [dirty])
  // Flash class kicks in after 5 minutes of unsaved edits.
  const FIVE_MIN = 5 * 60 * 1000
  const flashing = dirty && elapsed >= FIVE_MIN
  return (
    <button
      onClick={onClick}
      className={`fl-mini-btn${flashing ? ' flash' : ''}`}
      title={dirty ? 'Save as… (Ctrl+Shift+S) · unsaved changes pending' : 'Save as… (Ctrl+Shift+S)'}
    >
      <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.1" strokeLinejoin="round">
        <path d="M2 2h6l2 2v6H2zM4 2v3h4V2M4 10v-3h3"/>
      </svg>
    </button>
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
              <HwAddLaneButton trackId={t.id} onAdd={addAutomationLane} />
            </div>
          )
          // Render the track's automation lanes directly under it. The
          // add-lane affordance used to sit on its own 18 px row after
          // the lanes, but that broke canvas-grid alignment because the
          // grid assumes every row is `trackHeight`. The "+L" button on
          // the track header above opens the same lane-target picker
          // without taking any vertical space.
          const laneRows = t.automationLanes.map(lane => (
            <AutomationLane key={lane.id} trackId={t.id} lane={lane} />
          ))
          return [row, ...laneRows]
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

/**
 * Compact "+L" pill in the track header that opens the automation lane
 * target picker. Replaces the old full-width row button which sat between
 * tracks at 18 px tall — a height that broke alignment with the canvas
 * grid (canvas assumes every row is `trackHeight`). Moving the affordance
 * INTO the row header eliminates the off-grid row entirely.
 */
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
    <span style={{ position: 'relative', display: 'inline-flex' }}>
      <button
        type="button"
        className="fl-tr-arm"
        onClick={(e) => { e.stopPropagation(); setOpen((v) => !v) }}
        onMouseDown={(e) => e.stopPropagation()}
        title="Add automation lane"
        aria-label="Add an automation lane to this track"
        style={{ fontWeight: 600 }}
      >
        +L
      </button>
      {open && (
        <div
          className="fl-lane-target-pop"
          onMouseDown={(e) => e.stopPropagation()}
          style={{ position: 'absolute', top: 'calc(100% + 4px)', right: 0, zIndex: 20 }}
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
    </span>
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
  onTogglePlaylist,
  onOpenTempoTapper,
  onAction,
  onOpenExport,
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
  // Boot the rAF-driven performance meter sampler once for the
  // lifetime of the app. The cleanup teardown is fine on hot reload.
  useEffect(() => startPerfMeters(), [])
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
      <HwTopbar
        menus={menus}
        onTogglePlaylist={onTogglePlaylist}
        onToggleChannelRack={onToggleChannelRack}
        onOpenTempoTapper={onOpenTempoTapper}
        onAction={onAction}
        onOpenExport={onOpenExport}
      />
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
