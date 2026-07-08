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

import React, { useState, useCallback, useEffect, useMemo } from 'react'
import { Browser } from './browser/Browser'
import { Arrangement } from './arrangement/Arrangement'
import { ChannelRack } from './channelrack/ChannelRack'
import { PianoRoll } from './piano-roll/PianoRoll'
import { MixerPanel } from './mixer/MixerPanel'
import { HwTopMenu, type MenuDef } from './HwTopMenu'
import { AutomationLane } from './AutomationLane'
import { AutomationClipLane } from './AutomationClipLane'
import { KickSynthEditor } from './KickSynthEditor'
import type { AutomationTargetInfo } from '../stores/trackStore'
import { useTransportStore } from '../stores/transportStore'
import { useTrackStore } from '../stores/trackStore'
import { usePatternStore } from '../stores/patternStore'
import { usePickerStore } from '../stores/pickerStore'
import { usePlaylistToolStore, type PlaylistTool } from '../stores/playlistToolStore'
import { ArrangementSwitcher } from './ArrangementSwitcher'
import { usePanelLayoutStore } from '../stores/panelLayoutStore'
import { useHoverInfoStore } from '../stores/hoverInfoStore'
import { useProjectStore } from '../stores/projectStore'
import { useMetronomeStore } from '../stores/metronomeStore'
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

export function HwTopbar({
  menus, onTogglePlaylist, onToggleChannelRack, onTogglePianoRoll, onToggleMixer,
  showPlaylist, showChannelRack, showPianoRoll, showMixer,
  onOpenTempoTapper, onAction, onOpenExport,
}: {
  menus?: MenuDef[]
  onTogglePlaylist?: () => void
  onToggleChannelRack?: () => void
  onTogglePianoRoll?: () => void
  onToggleMixer?: () => void
  showPlaylist?: boolean
  showChannelRack?: boolean
  showPianoRoll?: boolean
  showMixer?: boolean
  onOpenTempoTapper?: () => void
  onAction?: (id: ActionId) => void
  onOpenExport?: () => void
}) {
  const playing = useTransportStore(s => s.playing)
  const recording = useTransportStore(s => s.recording)
  const looping = useTransportStore(s => s.looping)
  const bpm = useTransportStore(s => s.bpm)
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
  const waitForInput = useRecordingPrefsStore(s => s.waitForInput)
  const toggleWaitForInput = useRecordingPrefsStore(s => s.toggleWaitForInput)
  const blendRecord = useRecordingPrefsStore(s => s.blendRecord)
  const toggleBlendRecord = useRecordingPrefsStore(s => s.toggleBlendRecord)
  const typingKbdEnabled = useTypingKeyboardStore(s => s.enabled)
  const toggleTypingKbd = useTypingKeyboardStore(s => s.toggle)
  const precountBars = useMetronomeStore(s => s.precountBars)
  const setPrecountBars = useMetronomeStore(s => s.setPrecountBars)

  const { minSec } = useTransportClock()

  const patterns = usePatternStore(s => s.patterns)
  const activeId = usePatternStore(s => s.activeId)
  const activePattern = patterns.find(p => p.id === activeId)
  const prevPattern = usePatternStore(s => s.prevPattern)
  const nextPattern = usePatternStore(s => s.nextPattern)

  const metronomeEnabled = useMetronomeStore(s => s.enabled)
  const toggleMetronome = useMetronomeStore(s => s.toggleEnabled)


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
    // Defensive close with breadcrumbs into the DevPanel Log (the
    // Hardwave custom dev panel opens with Ctrl+Shift+D and reads
    // from `useLogStore`, not `useNotificationStore`). We also fire
    // a custom `daw:close-clicked` event so the DevPanel's daw:*
    // event subscription captures it, and console.log everywhere
    // for completeness. Whichever surface the user has open shows
    // the failing step.
    let log: (level: 'info' | 'fail' | 'pass' | 'event', msg: string) => void = () => {}
    try {
      const mod = await import('../dev/logStore')
      const append = mod.useLogStore.getState().append
      log = (level, message) => append({ level, message })
    } catch {}
    const breadcrumb = (level: 'info' | 'fail' | 'pass' | 'event', msg: string) => {
      try { console.log(`[close] ${msg}`) } catch {}
      try { log(level, msg) } catch {}
      try { window.dispatchEvent(new CustomEvent('daw:close-trace', { detail: { level, msg } })) } catch {}
    }
    breadcrumb('event', 'X clicked — entering close handler')
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      const win = getCurrentWindow()
      breadcrumb('info', 'got window — calling close()')
      const closedFlag = win.close().then(() => 'ok' as const).catch((e) => `err:${String(e)}`)
      const timeout = new Promise<'timeout'>(resolve => setTimeout(() => resolve('timeout'), 1500))
      const outcome = await Promise.race([closedFlag, timeout])
      breadcrumb(outcome === 'ok' ? 'pass' : 'fail', `close() outcome = ${outcome}`)
      if (outcome === 'timeout') {
        try {
          breadcrumb('info', 'trying destroy() fallback')
          await win.destroy()
          breadcrumb('pass', 'destroy() returned')
        } catch (e) {
          breadcrumb('fail', `destroy() failed — ${String(e)}`)
        }
      }
    } catch (e) {
      breadcrumb('fail', `unexpected error — ${String(e)}`)
    }
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
    {/* data-tauri-drag-region: the window ships decorations:false, so
        WITHOUT this the main window cannot be moved at all — a real
        regression the window-chrome Playwright spec caught (the old
        TitleBar had it; the HwApp port lost it). Tauri only starts a
        drag when the mousedown lands on the attributed element itself,
        so the menus/buttons inside keep working. */}
    <div className="fl-topbar" data-tauri-drag-region>
      <div className="fl-logo" data-tauri-drag-region>HARD<span>WAVE</span></div>
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
      {/* Panel access — FL-style F5/F6/F7/F9 toggles. Active = panel open. */}
      <div className="fl-panel-btns" role="toolbar" aria-label="Panels">
        <button
          className={`fl-panel-btn${showPlaylist ? ' on' : ''}`}
          onClick={() => onTogglePlaylist?.()}
          data-hint="Playlist (F5) — arrange clips on the timeline"
          title="Playlist (F5)"
        >Playlist</button>
        <button
          className={`fl-panel-btn${showChannelRack ? ' on' : ''}`}
          onClick={() => onToggleChannelRack?.()}
          data-hint="Channel Rack (F6) — step sequencer + instrument channels"
          title="Channel Rack (F6)"
        >Channels</button>
        <button
          className={`fl-panel-btn${showPianoRoll ? ' on' : ''}`}
          onClick={() => onTogglePianoRoll?.()}
          data-hint="Piano Roll (F7) — draw and edit notes for the selected channel"
          title="Piano Roll (F7)"
        >Piano Roll</button>
        <button
          className={`fl-panel-btn${showMixer ? ' on' : ''}`}
          onClick={() => onToggleMixer?.()}
          data-hint="Mixer (F9) — track levels, inserts, sends, routing"
          title="Mixer (F9)"
        >Mixer</button>
      </div>
      <div className="fl-topbar-spacer" />
      {/* Perf meters (CPU/MEM) + MIDI activity — Option B top-right cluster. */}
      <div className="fl-topbar-perf">
        <HwPerfCluster />
        <HwMidiActivityLed />
      </div>
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

      {/* BPM — two drag zones (integer + decimal), click for edit, RMB
          for menu. FL Studio parity: vertical drag on the integer
          portion of the readout adjusts whole BPM (step 1, Ctrl=fine
          0.1), drag on the decimal portion adjusts the fractional
          portion only (step 0.001). Click-to-edit + right-click menu
          live on the outer wrapper so the existing flow still works. */}
      <div style={{ position: 'relative' }}>
        <div
          className="fl-bpm"
          title="Tempo · click to edit · drag integer for whole BPM · drag decimal for thousandths · right-click for menu"
          onClick={(e) => {
            // Suppress click-to-edit if a child digit drag just
            // committed a new value — the per-zone handlers below
            // set this flag on the wrapper element to short-circuit.
            const w = e.currentTarget as HTMLElement & { dataset: DOMStringMap }
            if (w.dataset.justDragged === '1') {
              w.dataset.justDragged = '0'
              return
            }
            if (!editingBpm) {
              setBpmDraft(bpm.toFixed(3))
              setEditingBpm(true)
            }
          }}
          onContextMenu={(e) => { e.preventDefault(); setTempoMenuOpen(v => !v) }}
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
            <HwBpmSplitDisplay bpm={bpm} setBpm={setBpm} />
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
        style={{ width: 'auto', height: 30, padding: '0 11px', fontSize: 10, fontWeight: 700, letterSpacing: 0.4, fontFamily: 'var(--mono)' }}
        title="Tap tempo · right-click opens Tempo Tapper modal"
      >TAP</button>

      <span className="fl-toolsep" />

      {/* Clock — Min:Sec only */}
      <div className="fl-clock" title="Playhead position">
        <div className="fl-clock-stack red">
          <small>MIN : SEC</small>
          <b>{minSec}</b>
        </div>
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

      {/* Recording toggle cluster. Step-editing / multilink stay
          hidden until their backends exist (removed 2026-07-07,
          deep-research P1-6 — buttons that do nothing read as
          broken). Wait-for-input + blend-record RETURNED 2026-07-08
          fully wired (engine wait_for_input_tests + blend_tests). */}
      <div className="fl-action-row">
        <button
          onClick={() => toggleBlendRecord()}
          className={`fl-mini-btn${blendRecord ? ' on' : ''}`}
          title="Blend / overdub record (Ctrl+B) — new takes merge into the existing clip"
        >
          <svg className="ic" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1">
            <circle cx="5" cy="6" r="3"/>
            <circle cx="8" cy="6" r="3"/>
          </svg>
        </button>
        <button
          onClick={() => toggleWaitForInput()}
          className={`fl-mini-btn${waitForInput ? ' on' : ''}`}
          title="Wait for input (Ctrl+I) — Play/Record start on your first MIDI note"
        >
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
        <button onClick={() => toggleTypingKbd()} className={`fl-mini-btn${typingKbdEnabled ? ' on' : ''}`} title="Typing keyboard → piano (Ctrl+T)">
          <svg className="ic" width="13" height="9" viewBox="0 0 13 9" fill="none" stroke="currentColor" strokeWidth="0.8">
            <rect x="0.5" y="0.5" width="12" height="8" rx="1"/>
            <rect x="2" y="2" width="2" height="2" rx="0.3" fill="currentColor"/>
            <rect x="5.5" y="2" width="2" height="2" rx="0.3" fill="currentColor"/>
            <rect x="9" y="2" width="2" height="2" rx="0.3" fill="currentColor"/>
            <rect x="3" y="5.5" width="7" height="1.5" rx="0.3" fill="currentColor" opacity="0.6"/>
          </svg>
        </button>
      </div>

    </div>
    </>
  )
}

// ─── BPM split-display (FL per-zone drag) ──────────────────────────────────
//
// Renders the BPM readout as two independently draggable spans
// separated by a static dot. Vertical drag on the integer span
// adjusts whole BPM (step 1, Ctrl/Cmd = fine 0.1); drag on the
// decimal span adjusts the fractional portion only (step 0.001).
// FL Studio's tempo display works the same — sleep over a digit and
// only that digit's place value moves. Click-to-edit lives on the
// parent `.fl-bpm` div so a bare click anywhere still opens the
// numeric editor.

function HwBpmSplitDisplay({ bpm, setBpm }: { bpm: number; setBpm: (v: number) => void }) {
  const intPart = Math.floor(bpm)
  const decPart = bpm - intPart // 0..0.999
  const decStr = (Math.round(decPart * 1000) / 1000).toFixed(3).slice(2) // '000'..'999'

  // Walk up the DOM to the `.fl-bpm` wrapper and stamp a flag so the
  // parent's onClick handler skips its click-to-edit branch on the
  // pointerup that immediately follows a drag.
  const markDragged = (el: HTMLElement) => {
    let n: HTMLElement | null = el
    while (n && !n.classList.contains('fl-bpm')) n = n.parentElement
    if (n) (n as HTMLElement & { dataset: DOMStringMap }).dataset.justDragged = '1'
  }

  const handleIntDrag = (e: React.PointerEvent<HTMLSpanElement>) => {
    if (e.button !== 0) return
    e.stopPropagation()
    const startY = e.clientY
    const startInt = intPart
    const startDec = decPart
    const target = e.currentTarget
    target.setPointerCapture(e.pointerId)
    let moved = false
    const onMove = (ev: PointerEvent) => {
      const dy = startY - ev.clientY
      if (Math.abs(dy) > 2) moved = true
      const fine = ev.ctrlKey || ev.metaKey ? 0.1 : 1
      const stepped = Math.round(startInt + dy * 0.3 * fine)
      const nextInt = Math.max(20, Math.min(999, stepped))
      setBpm(nextInt + startDec)
    }
    const onUp = (ev: PointerEvent) => {
      target.releasePointerCapture(ev.pointerId)
      target.removeEventListener('pointermove', onMove)
      target.removeEventListener('pointerup', onUp)
      if (moved) markDragged(target)
    }
    target.addEventListener('pointermove', onMove)
    target.addEventListener('pointerup', onUp)
  }

  const handleDecDrag = (e: React.PointerEvent<HTMLSpanElement>) => {
    if (e.button !== 0) return
    e.stopPropagation()
    const startY = e.clientY
    const startInt = intPart
    const startDec = decPart
    const target = e.currentTarget
    target.setPointerCapture(e.pointerId)
    let moved = false
    const onMove = (ev: PointerEvent) => {
      const dy = startY - ev.clientY
      if (Math.abs(dy) > 2) moved = true
      const fine = ev.ctrlKey || ev.metaKey ? 0.1 : 1
      // 0.005 BPM per pixel × fine — full screen drag = ~5 BPM.
      const rawDec = Math.max(0, Math.min(0.999, startDec + dy * 0.005 * fine))
      const nextDec = Math.round(rawDec * 1000) / 1000
      setBpm(startInt + nextDec)
    }
    const onUp = (ev: PointerEvent) => {
      target.releasePointerCapture(ev.pointerId)
      target.removeEventListener('pointermove', onMove)
      target.removeEventListener('pointerup', onUp)
      if (moved) markDragged(target)
    }
    target.addEventListener('pointermove', onMove)
    target.addEventListener('pointerup', onUp)
  }

  const zoneStyle: React.CSSProperties = {
    cursor: 'ns-resize',
    padding: '0 1px',
    borderRadius: 2,
    userSelect: 'none',
    fontFamily: 'var(--mono)',
    fontSize: 14,
    fontWeight: 700,
  }
  return (
    <b style={{ display: 'inline-flex', alignItems: 'baseline', fontFamily: 'var(--mono)' }}>
      <span
        onPointerDown={handleIntDrag}
        style={zoneStyle}
        title="Drag to adjust whole BPM"
      >{intPart}</span>
      <span style={{ ...zoneStyle, cursor: 'default', padding: 0, opacity: 0.5 }}>.</span>
      <span
        onPointerDown={handleDecDrag}
        style={zoneStyle}
        title="Drag to adjust thousandths"
      >{decStr}</span>
    </b>
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

export function HwSecondRow({ projectName }: { projectName: string }) {
  // Live hover info, fed by the delegated listener in HwApp.
  const hint = useHoverInfoStore(s => s.info)
  // Ship 3c — Hint Bar redesign. The legacy fl-tag-pill / fl-step-pill
  // row duplicated controls now living on the toolbar (snap pill,
  // time-sig, recording state). The new row mirrors FL's hint bar:
  // an icon strip on the left telling the user what kind of object
  // the hint is about (REC / MIDI / right-mouse-affordance / sad
  // error / clock / sync), the live hint string in the middle,
  // and a SYNC LED on the right that pulses on bar / beat starts.
  const recording = useTransportStore(s => s.recording)
  const playing = useTransportStore(s => s.playing)
  const tsNum = useTransportStore(s => s.timeSigNumerator)
  const tsDen = useTransportStore(s => s.timeSigDenominator)
  const positionSamples = useTransportStore(s => s.positionSamples)
  const sampleRate = useTransportStore(s => s.sampleRate)
  const bpm = useTransportStore(s => s.bpm)

  // Compute whether the playhead just crossed a beat boundary so the
  // sync LED can blink in time. Beats-per-second = bpm/60; a beat
  // boundary is when (positionSamples / sampleRate / beatsPerSec) is
  // within one frame of an integer.
  const seconds = sampleRate > 0 ? positionSamples / sampleRate : 0
  const beats = bpm > 0 ? (seconds * bpm / 60) : 0
  const beatFrac = beats - Math.floor(beats)
  const onBeat = playing && (beatFrac < 0.05 || beatFrac > 0.95)
  const onBar = playing && onBeat && Math.floor(beats) % Math.max(1, tsNum) === 0

  const defaultHint = `${projectName}  ·  Hover anything for live info`

  // Lightweight heuristic for the icon type — pick the highest-
  // priority badge that applies right now. The full FL set (sad /
  // happy / clock / fast-forward / rewind / left-arrow) joins
  // when the hint origin emits a category, which is a Ship 4 ask.
  const showRecIcon = recording
  const showMidiIcon = hint.toLowerCase().includes('midi')
  const showRmbIcon = hint.toLowerCase().includes('right-click') || hint.toLowerCase().includes('rmb')

  return (
    <div className="fl-second-row fl-hint-row">
      <div className="fl-hint-icons">
        {showRecIcon && (
          <span className="fl-hint-icon rec" title="Recording is armed">
            <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.2">
              <circle cx="6" cy="6" r="4.5" />
              <circle cx="6" cy="6" r="1.6" fill="currentColor" />
            </svg>
          </span>
        )}
        {showMidiIcon && (
          <span className="fl-hint-icon midi" title="MIDI control available">
            <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.1">
              <circle cx="6" cy="6" r="4"/>
              <circle cx="6" cy="6" r="1" fill="currentColor"/>
              <line x1="6" y1="2" x2="6" y2="4"/>
            </svg>
          </span>
        )}
        {showRmbIcon && (
          <span className="fl-hint-icon rmb" title="Right-click affordance">
            <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.1">
              <path d="M6 2a3 3 0 0 0-3 3v3a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3z"/>
              <path d="M6 2v3.5h2.5" fill="currentColor"/>
            </svg>
          </span>
        )}
      </div>
      <span className={`fl-hint${hint ? ' active' : ''}`}>
        {hint || defaultHint}
      </span>
      <div className="fl-hint-sync" title="Transport sync (pulses on beat / bar starts)">
        <span className={`led${onBar ? ' on-bar' : onBeat ? ' on-beat' : ''}`} />
        <span className="label">SYNC</span>
      </div>
      <div className="fl-hint-tsig" title={`Time signature ${tsNum}/${tsDen}`}>{tsNum}/{tsDen}</div>
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
  // Live tool binding — these buttons were decorative until 2026-07-07
  // ("Select" hard-coded active, clicks did nothing) while only the
  // keyboard shortcuts drove the real store Arrangement.tsx reads.
  const activeTool = usePlaylistToolStore(s => s.tool)
  const setTool = usePlaylistToolStore(s => s.setTool)

  const tools: Array<{ id: PlaylistTool; title: string; icon: React.ReactNode }> = [
    { id: 'select', title: 'Select tool (S)', icon: (
      <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
        <path d="M3 2.5l8.5 4.5-4.2 1.6L5.5 13z" fill="currentColor" stroke="currentColor" strokeWidth=".8" strokeLinejoin="round"/>
      </svg>
    )},
    { id: 'draw', title: 'Draw tool (P)', icon: (
      <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
        <path d="M2 14l1-3 8-8 2 2-8 8z M10 4l2 2" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round"/>
      </svg>
    )},
    { id: 'paint', title: 'Paint tool (B)', icon: (
      <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
        <path d="M9 4l3 3-5 5c-1 1-3 1-3 0s1-1.5 1-3z M9 4l3-2 2 2-2 3z" fill="currentColor" stroke="currentColor" strokeWidth=".8" strokeLinejoin="round"/>
      </svg>
    )},
    { id: 'slice', title: 'Slice tool (C)', icon: (
      <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
        <circle cx="4" cy="11" r="2" fill="none" stroke="currentColor" strokeWidth="1.2"/>
        <circle cx="12" cy="11" r="2" fill="none" stroke="currentColor" strokeWidth="1.2"/>
        <path d="M5.5 9.5L13 2.5M10.5 9.5L3 2.5" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round"/>
      </svg>
    )},
    { id: 'mute', title: 'Mute tool (T)', icon: (
      <svg className="ic" width="13" height="13" viewBox="0 0 16 16" fill="none">
        <path d="M3 6.5h2l3-2.5v8L5 9.5H3z" fill="currentColor"/>
        <path d="M11 5l4 6M15 5l-4 6" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round"/>
      </svg>
    )},
    { id: 'delete', title: 'Delete tool (D)', icon: (
      <svg className="ic" width="12" height="12" viewBox="0 0 16 16" fill="none">
        <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/>
      </svg>
    )},
  ]

  return (
    <div className="fl-pl-tools">
      {tools.map(t => (
        <button
          key={t.id}
          type="button"
          className={`fl-tool${activeTool === t.id ? ' on' : ''}`}
          onClick={() => setTool(t.id)}
          title={t.title}
        >
          {t.icon}
        </button>
      ))}
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
  const createAutomationClip = useTrackStore(s => s.createAutomationClip)
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
        {tracks.flatMap((t) => {
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
              <button
                type="button"
                title="Add automation clip (Volume, 4 bars — edit target/length after)"
                onClick={(e) => {
                  e.stopPropagation()
                  // Default: a 4-bar volume automation clip at the start
                  // (4 bars × 4 beats × 960 PPQ = 15360 ticks).
                  createAutomationClip(t.id, { kind: 'track_volume' }, 0, 15360)
                }}
                style={{
                  fontSize: 9, fontWeight: 700, padding: '0 4px', marginLeft: 2,
                  background: 'rgba(255,255,255,0.06)', color: '#bdbdc8',
                  border: '1px solid #2a2a36', borderRadius: 4, cursor: 'pointer',
                }}
              >
                +A
              </button>
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
          const clipRows = (t.automationClips ?? []).map(clip => (
            <AutomationClipLane key={clip.id} trackId={t.id} clip={clip} />
          ))
          return [row, ...laneRows, ...clipRows]
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
  { id: 'builtin_sine',     label: 'Sine (default)', abbr: 'SIN' },
  { id: 'builtin_saw',      label: 'Saw',            abbr: 'SAW' },
  { id: 'builtin_square',   label: 'Square',         abbr: 'SQR' },
  { id: 'builtin_triangle', label: 'Triangle',       abbr: 'TRI' },
  { id: 'kick_synth',       label: 'KickSynth',      abbr: 'KIK' },
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
        Playlist ·
        {/* Switchable arrangements (FL 20+ parity). The component +
            backend commands existed end-to-end but were surfaced
            NOWHERE — this line is what actually ships the feature. */}
        <ArrangementSwitcher />
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
  // "Hover anything for live info": a delegated mouseover listener reads
  // the nearest element's data-hint (preferred) or title and feeds the
  // global hover-info store, which the status strip renders. Every control
  // that already has a `title` lights up the info bar for free.
  useEffect(() => {
    const onOver = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null
      const el = target?.closest('[data-hint],[title]') as HTMLElement | null
      const info = el
        ? el.getAttribute('data-hint') || el.getAttribute('title') || ''
        : ''
      useHoverInfoStore.getState().setInfo(info)
    }
    document.addEventListener('mouseover', onOver)
    return () => document.removeEventListener('mouseover', onOver)
  }, [])
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

  // Mobile: single panel, minimal chrome. The slim grab strip keeps the
  // window movable — decorations are off, and a desktop window resized
  // this narrow flips into this layout, so without a drag region it
  // would become permanently unmovable.
  if (isMobile) {
    return (
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', minHeight: 0 }}>
        <div
          data-tauri-drag-region
          style={{ height: 14, flexShrink: 0, background: 'rgba(255,255,255,0.03)', cursor: 'grab' }}
          title="Drag to move window"
        />
        <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 0 }}>
          {mobilePanel === 'browser' && <Browser />}
          {mobilePanel === 'channelRack' && <ChannelRack />}
          {mobilePanel === 'pianoRoll' && <PianoRoll />}
          {mobilePanel === 'playlist' && <Arrangement />}
          {mobilePanel === 'mixer' && <MixerPanel />}
        </div>
      </div>
    )
  }

  return (
    <div className="fl-app" data-testid="hw-app">
      <HwTopbar
        menus={menus}
        onTogglePlaylist={onTogglePlaylist}
        onToggleChannelRack={onToggleChannelRack}
        onTogglePianoRoll={onTogglePianoRoll}
        onToggleMixer={onToggleMixer}
        showPlaylist={showPlaylist}
        showChannelRack={showChannelRack}
        showPianoRoll={showPianoRoll}
        showMixer={showMixer}
        onOpenTempoTapper={onOpenTempoTapper}
        onAction={onAction}
        onOpenExport={onOpenExport}
      />
      <HwSecondRow projectName={projectName} />

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
                onMouseLeave={() => useHoverInfoStore.getState().clearInfo()}
                style={{ ['--row-h' as any]: `${trackHeight}px` }}
              >
                <HwPlaylistTracks />
                <div className="fl-pl-grid">
                  <HwPlaylistRuler />
                  <div className="fl-pl-canvas">
                    <Arrangement onSetHint={(s) => useHoverInfoStore.getState().setInfo(s)} />
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
