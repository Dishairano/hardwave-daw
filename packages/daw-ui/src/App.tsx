import { useEffect, useState, useRef, useCallback, useMemo } from 'react'
import { SplashScreen } from './components/SplashScreen'
import { TitleBar } from './components/transport/TitleBar'
import { Toolbar } from './components/transport/Toolbar'
import { HwApp } from './components/HwApp'
import type { MenuDef, MenuItem } from './components/HwTopMenu'
import { usePatternStore } from './stores/patternStore'
import { useUiPreferencesStore, UI_SCALE_OPTIONS, type UiScale } from './stores/uiPreferencesStore'
import { TrackList } from './components/arrangement/TrackList'
import { Arrangement } from './components/arrangement/Arrangement'
import { MixerPanel } from './components/mixer/MixerPanel'
import { Browser } from './components/browser/Browser'
import { ChannelRack } from './components/channelrack/ChannelRack'
import { PianoRoll } from './components/piano-roll/PianoRoll'
import { Roadmap } from './components/roadmap/Roadmap'
import { AudioSettings } from './components/settings/AudioSettings'
import { ThemePicker } from './components/settings/ThemePicker'
import { useMixerSettingsStore } from './stores/mixerSettingsStore'
import { usePlaylistToolStore } from './stores/playlistToolStore'
import { useMarkerStore } from './stores/markerStore'
import { ColorPicker } from './components/primitives/ColorPicker'
import { useColorPickerStore } from './stores/colorPickerStore'
import { UpdateModal } from './components/UpdateModal'
import { AboutDialog } from './components/AboutDialog'
import { FloatingWindow } from './components/FloatingWindow'
import { SaveChangesDialog, type SaveChangesChoice } from './components/SaveChangesDialog'
import { TemplateDialog, type TemplateId } from './components/TemplateDialog'
import { ExportDialog } from './components/ExportDialog'
import { useUserTemplateStore } from './stores/userTemplateStore'
import { useTrackTemplateStore } from './stores/trackTemplateStore'
import { TrackTemplateManager } from './components/TrackTemplateManager'
import { WelcomeScreen, shouldSkipWelcome } from './components/WelcomeScreen'
import { NotificationHost } from './components/NotificationHost'
import { MetronomeScheduler } from './components/transport/MetronomeScheduler'
import { CrashRecoveryDialog, type CrashChoice } from './components/CrashRecoveryDialog'
import { ShortcutsPanel } from './components/ShortcutsPanel'
import { HelpOverlay } from './components/HelpOverlay'
import { SampleEditor } from './components/sample-editor/SampleEditor'
import { useSampleEditorStore } from './stores/sampleEditorStore'
import { BeatSlicer } from './components/beat-slicer/BeatSlicer'
import { useBeatSlicerStore } from './stores/beatSlicerStore'
import { LoudnessMeter } from './components/LoudnessMeter'
import { Oscilloscope } from './components/Oscilloscope'
import { SpectrumAnalyzer } from './components/SpectrumAnalyzer'
import { VirtualKeyboard } from './components/transport/VirtualKeyboard'
import './components/transport/VirtualKeyboard.css'
import { SetupWizard } from './components/SetupWizard'
import './components/SetupWizard.css'
import { ProjectInfoDialog } from './components/ProjectInfoDialog'
import { TempoTapper } from './components/TempoTapper'
import { maybeAutoOpenSetupWizard, useSetupWizardStore } from './stores/setupWizardStore'
import {
  AUTOSAVE_OPTIONS,
  frequencyIntervalMs,
  useAutosavePrefsStore,
} from './stores/autosavePrefsStore'
import { useGeneralPrefsStore } from './stores/generalPrefsStore'
import { MidiMappingsPanel, type MidiMapTarget } from './components/MidiMappingsPanel'
import { TempoMapDialog } from './components/TempoMapDialog'
import { HistoryPanel } from './components/HistoryPanel'
import { PrecountOverlay } from './components/transport/PrecountOverlay'
import { invoke } from '@tauri-apps/api/core'
import { usePanelLayoutStore } from './stores/panelLayoutStore'
import { useIsMobile } from './hooks/useIsMobile'
import { MobileTabBar, type MobilePanel } from './components/MobileTabBar'
import { DevPanel } from './dev/DevPanel' // DEV ONLY — remove before merge to master
import { useTransportStore } from './stores/transportStore'
import { useTrackStore } from './stores/trackStore'
import { usePluginStore } from './stores/pluginStore'
import { useProjectStore } from './stores/projectStore'
import { useShortcutsStore } from './stores/shortcutsStore'
import { useComputerMidiKeyboard } from './hooks/useComputerMidiKeyboard'
import { useTypingKeyboardStore } from './stores/typingKeyboardStore'
import { useMetronomeStore } from './stores/metronomeStore'
import { useTouchControllerStore } from './stores/touchControllerStore'
import { useNotificationStore } from './stores/notificationStore'
import { applyCustomBg, useThemeStore } from './stores/themeStore'
import { hw } from './theme'

interface UpdateInfo {
  version: string
  changelog: string
  date: string | null
  available: boolean
  dismissed: boolean
  downloading: boolean
  progress: number
  downloaded: boolean
  error: string | null
  /**
   * Optional GitHub release URL surfaced as a "Open release notes" fallback
   * link inside the update-error branch of UpdateModal. Sourced from the
   * manifest's `installer_hint.release_url` via `version_contract_state`.
   */
  releaseUrl?: string | null
}

/**
 * Mirrors `VersionContractState` in `src-tauri/src/frontend_updater.rs`.
 * Single source of truth for the launch-time decision between the
 * Tauri auto-updater (Path A) and the splash-driven hot-swap (Path B).
 */
type VersionContractDecision =
  | 'hot_swap'
  | 'installer_required'
  | 'up_to_date'
  | 'fallback'

interface VersionContractState {
  decision: VersionContractDecision
  installer_target?: string | null
  installer_track?: 'stable' | 'beta'
  release_url?: string | null
  hot_swap_version?: string | null
  reason?: string | null
  known_schema?: number
}

/** Compact byte size for the splash status text. */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function shortenRecentPath(p: string): string {
  const parts = p.replace(/\\/g, '/').split('/')
  const name = parts[parts.length - 1] || p
  return name.length > 40 ? name.slice(0, 37) + '...' : name
}

export function App() {
  const { startListening } = useTransportStore()
  const { fetchTracks } = useTrackStore()
  const { newProject, saveProject, loadProject } = useProjectStore()
  const recentProjects = useProjectStore(s => s.recentProjects)
  const uiScaleMode = useUiPreferencesStore(s => s.mode)
  const uiScaleEffective = useUiPreferencesStore(s => s.effectiveScale)
  const setUiScaleMode = useUiPreferencesStore(s => s.setUiScaleMode)

  // QWERTY-as-MIDI-keyboard. Subscribes to the persisted toggle so a
  // user flip in the toolbar instantly enables/disables note injection.
  const typingKeyboardEnabled = useTypingKeyboardStore((s) => s.enabled)
  useComputerMidiKeyboard({ enabled: typingKeyboardEnabled })

  // Reactive subscriptions for Options menu state mirrors — when the
  // user toggles via shortcut, the menu checkmarks update on next open.
  const metronomeEnabled = useMetronomeStore((s) => s.enabled)

  // Apply System Settings → General DOM classes. Animations toggle
  // hangs `.hw-no-animations` on <html> so the global stylesheet can
  // short-circuit transitions for every element below. High-visibility
  // adds `.hw-high-vis` for the a11y contrast layer.
  const animationsEnabled = useGeneralPrefsStore((s) => s.animationsEnabled)
  const highVisibility = useGeneralPrefsStore((s) => s.highVisibility)
  useEffect(() => {
    document.documentElement.classList.toggle('hw-no-animations', !animationsEnabled)
    document.documentElement.classList.toggle('hw-high-vis', highVisibility)
  }, [animationsEnabled, highVisibility])

  // Project working-time counter: every 30 seconds while the window has
  // focus, bump the project metadata counter by 30 via tick_project_working_time.
  // The Project Info dialog reads + can reset this.
  useEffect(() => {
    let lastTick = Date.now()
    const id = setInterval(() => {
      if (document.hidden) return
      const elapsed = Math.round((Date.now() - lastTick) / 1000)
      lastTick = Date.now()
      if (elapsed > 0) {
        invoke('tick_project_working_time', { seconds: elapsed }).catch(() => {})
      }
    }, 30_000)
    return () => clearInterval(id)
  }, [])

  // Auto-open Project Info splash when a freshly-loaded project has
  // show_on_open enabled. Runs once per project load — uses a one-shot
  // ref so reopening the dialog manually doesn't trigger a loop.
  const projectInfoCheckedRef = useRef('')
  useEffect(() => {
    const path = useProjectStore.getState().filePath ?? ''
    if (projectInfoCheckedRef.current === path) return
    projectInfoCheckedRef.current = path
    invoke<{ show_on_open: boolean }>('get_project_meta')
      .then(meta => { if (meta.show_on_open) setShowProjectInfo(true) })
      .catch(() => {})
  })

  // Panel visibility
  const [showBrowser, setShowBrowser] = useState(true)
  const [showMixer, setShowMixer] = useState(false)
  const [showChannelRack, setShowChannelRack] = useState(false)
  const [showPlaylist, setShowPlaylist] = useState(true)
  const [showPianoRoll, setShowPianoRoll] = useState(false)
  const [showRoadmap, setShowRoadmap] = useState(false)
  const [showAudioSettings, setShowAudioSettings] = useState(false)
  const [showThemePicker, setShowThemePicker] = useState(false)
  const [showAbout, setShowAbout] = useState(false)
  const [showShortcuts, setShowShortcuts] = useState(false)
  const [showHelp, setShowHelp] = useState(false)
  const [showTrackTemplateManager, setShowTrackTemplateManager] = useState(false)
  const [showLoudness, setShowLoudness] = useState(false)
  const [showOscilloscope, setShowOscilloscope] = useState(false)
  const [showSpectrum, setShowSpectrum] = useState(false)
  const [showProjectInfo, setShowProjectInfo] = useState(false)
  const [showTempoTapper, setShowTempoTapper] = useState(false)
  const [showMidiMappings, setShowMidiMappings] = useState(false)
  const [showTempoMap, setShowTempoMap] = useState(false)
  // Touch Controllers visibility is store-backed so View menu, Alt+F7
  // shortcut, and the close button all share state and the panel
  // remembers its open/closed status across reloads.
  const touchControllerVisible = useTouchControllerStore((s) => s.visible)
  const setTouchControllerVisible = useTouchControllerStore((s) => s.setVisible)
  const toggleTouchController = useTouchControllerStore((s) => s.toggleVisible)
  const [pdcEnabled, setPdcEnabled] = useState(true)
  const useNewMixer = useMixerSettingsStore(s => s.useNewMixer)
  const setUseNewMixer = useMixerSettingsStore(s => s.setUseNewMixer)
  const [midiLearnPreset, setMidiLearnPreset] = useState<MidiMapTarget | undefined>(undefined)
  const [showHistory, setShowHistory] = useState(false)
  const sampleEditorPath = useSampleEditorStore(s => s.openPath)
  const closeSampleEditor = useSampleEditorStore(s => s.close)
  const beatSlicerPath = useBeatSlicerStore(s => s.openPath)
  const closeBeatSlicer = useBeatSlicerStore(s => s.close)
  const [showDevPanel, setShowDevPanel] = useState(false) // DEV ONLY

  // Block browser-default chrome that leaks through Tauri's webview: the
  // right-click context menu (save / print / refresh) and the standard
  // reload + devtools keybindings (Ctrl+R, F5, F12, Ctrl+Shift+I/J/C, etc).
  // We keep INPUT / TEXTAREA / SELECT untouched so users can still right-
  // click for paste and use Ctrl+A/C/V/X/Z inside text fields. The dev
  // shortcut Ctrl+Shift+D is the single intentional debug entry point and
  // is wired in the keydown handler further down — it stays untouched.
  useEffect(() => {
    const isFormElement = (target: EventTarget | null): boolean => {
      const el = target as HTMLElement | null
      if (!el || !el.tagName) return false
      const tag = el.tagName
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true
      if (el.isContentEditable) return true
      return false
    }

    const blockContextMenu = (e: MouseEvent) => {
      if (isFormElement(e.target)) return
      e.preventDefault()
    }

    const blockBrowserKeys = (e: KeyboardEvent) => {
      const cmd = e.ctrlKey || e.metaKey
      // Reload variants — never useful in production.
      if (cmd && (e.key === 'r' || e.key === 'R')) { e.preventDefault(); return }
      if (e.key === 'F5') { e.preventDefault(); return }
      // Devtools — we have Ctrl+Shift+D for the in-app DevPanel.
      if (e.key === 'F12') { e.preventDefault(); return }
      if (cmd && e.shiftKey && (e.key === 'I' || e.key === 'i' || e.key === 'J' || e.key === 'j' || e.key === 'C' || e.key === 'c')) {
        // Ctrl+Shift+C is "inspect element" in browsers but also "copy" in
        // some text contexts — only block when no form element is focused.
        if (!isFormElement(e.target)) { e.preventDefault(); return }
      }
      // Print / view-source — produce useless output for a DAW window.
      if (cmd && (e.key === 'p' || e.key === 'P')) { e.preventDefault(); return }
      if (cmd && (e.key === 'u' || e.key === 'U')) { e.preventDefault(); return }
    }

    // Capture phase so we run before any user-bound shortcut, but we don't
    // stopPropagation — the app's own keydown handler still fires for
    // shortcuts like save/cut/paste/undo at bubble phase.
    window.addEventListener('contextmenu', blockContextMenu, { capture: true })
    window.addEventListener('keydown', blockBrowserKeys, { capture: true })
    return () => {
      window.removeEventListener('contextmenu', blockContextMenu, { capture: true } as EventListenerOptions)
      window.removeEventListener('keydown', blockBrowserKeys, { capture: true } as EventListenerOptions)
    }
  }, [])

  // Splash screen
  const [showSplash, setShowSplash] = useState(true)
  // Splash gate is split into two parallel readiness flags. Both must flip
  // true before the splash dismisses. `tracksReady` is set after the
  // existing fetchTracks() call; `frontendUpdateReady` is set after the
  // Rust frontend updater finishes (success, no-op, timeout, or error).
  const [tracksReady, setTracksReady] = useState(false)
  const [frontendUpdateReady, setFrontendUpdateReady] = useState(false)
  const dataReady = tracksReady && frontendUpdateReady
  // One-line message rendered under the splash loading bar — kept here so
  // it survives the splash component's lifecycle. Starts as a friendly
  // default the user sees before the updater has anything to report.
  const [splashStatus, setSplashStatus] = useState('Starting up...')

  // Welcome screen — shown on startup unless user has ticked "Don't show again"
  // (persistent) or already dismissed in this session.
  const [showWelcome, setShowWelcome] = useState(() => {
    if (shouldSkipWelcome()) return false
    try { return sessionStorage.getItem('hardwave.daw.welcomeDismissed') !== '1' } catch { return true }
  })
  const dismissWelcome = useCallback(() => {
    setShowWelcome(false)
    try { sessionStorage.setItem('hardwave.daw.welcomeDismissed', '1') } catch {}
  }, [])

  // Crash recovery
  const [crashInfo, setCrashInfo] = useState<{ path: string; modified_unix: number } | null>(null)

  // Export dialog
  const [showExport, setShowExport] = useState(false)

  // Mobile: which panel is currently visible (only one at a time).
  const isMobile = useIsMobile()
  const [mobilePanel, setMobilePanel] = useState<MobilePanel>('playlist')
  const selectMobilePanel = useCallback((panel: MobilePanel) => {
    setMobilePanel(panel)
    // Keep the toggle flags in sync so panel logic and piano-roll/etc. initializers still work.
    setShowBrowser(panel === 'browser')
    setShowPlaylist(panel === 'playlist')
    setShowChannelRack(panel === 'channelRack')
    setShowPianoRoll(panel === 'pianoRoll')
    setShowMixer(panel === 'mixer')
  }, [])

  // Open Piano Roll on request from arrangement double-click.
  useEffect(() => {
    const onOpen = () => setShowPianoRoll(true)
    window.addEventListener('daw:openPianoRoll', onOpen)
    return () => window.removeEventListener('daw:openPianoRoll', onOpen)
  }, [])

  useEffect(() => {
    const onOpen = (e: Event) => {
      const target = (e as CustomEvent<MidiMapTarget>).detail
      setMidiLearnPreset(target)
      setShowMidiMappings(true)
    }
    window.addEventListener('daw:openMidiLearn', onOpen)
    return () => window.removeEventListener('daw:openMidiLearn', onOpen)
  }, [])

  // When Piano Roll opens with no active clip but a MIDI track is selected,
  // ensure a default clip exists and bind it.
  useEffect(() => {
    if (!showPianoRoll) return
    const ts = useTrackStore.getState()
    if (ts.activeMidiClipId) return
    const trackId = ts.selectedTrackId
    if (!trackId) return
    const track = ts.tracks.find(t => t.id === trackId)
    if (!track || track.kind !== 'Midi') return
    ts.ensureMidiClipOnTrack(trackId).then(clipId => {
      if (clipId) ts.setActiveMidiClip(trackId, clipId)
    }).catch(() => {})
  }, [showPianoRoll])

  // Hint bar text
  const [hintText, setHintText] = useState('')

  // Update state — matches Suite pattern
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo>({
    version: '',
    changelog: '',
    date: null,
    available: false,
    dismissed: false,
    downloading: false,
    progress: 0,
    downloaded: false,
    error: null,
    releaseUrl: null,
  })
  const initRan = useRef(false)

  const customBg = useThemeStore(s => s.customBg)
  useEffect(() => { applyCustomBg(customBg) }, [customBg])

  useEffect(() => {
    if (initRan.current) return
    initRan.current = true
    startListening()
    // Auto-open the MIDI Setup Wizard once on first boot. The store
    // tracks completedFirstRun + skippedAt so this is a no-op after
    // the user has finished or skipped. The Help menu has a manual
    // "Re-run setup wizard" entry to reopen it on demand.
    maybeAutoOpenSetupWizard()

    // Cancellation flag matches the pattern used at :397-402 below — if
    // the splash is dismissed (or the effect tears down) before the
    // async resolver query resolves, we must not call setSplashStatus /
    // setUpdateInfo on an unmounted component. Mirrored on cleanup.
    let cancelled = false

    // Splash gate part 1 — load track state for the project AND start the
    // audio engine. Without start_engine the cpal output stream is never
    // opened so play() just flips a flag and no audio comes out, no time
    // advances. Bug regressed silently because only test code was
    // invoking start_engine. Wire it on every launch here.
    invoke('start_engine').catch((err) => {
      console.error('start_engine failed at boot:', err)
      useNotificationStore.getState().push('error', 'Audio engine failed to start', { detail: String(err) })
    })
    fetchTracks().finally(() => setTracksReady(true))

    // Block WebView2's default file-drop behaviour at the document
    // capture phase so dropped files never navigate the page to the
    // file URL (which is what was making the dragged audio "open in
    // the browser first"). Tauri's interceptor still imports the file
    // via the daw:drag-drop / tauri://drag-drop channels — this just
    // makes sure the WebView's default handler can never claim it.
    const blockDefaultDrag = (e: DragEvent) => { e.preventDefault() }
    document.addEventListener('dragenter', blockDefaultDrag, { capture: true })
    document.addEventListener('dragover', blockDefaultDrag, { capture: true })
    document.addEventListener('drop', blockDefaultDrag, { capture: true })

    // Splash gate part 2 — run the frontend updater. The Rust side is
    // budgeted (5s total) so this always resolves; on error we still set
    // ready so the splash can advance. Status events from the Rust side
    // pipe into setSplashStatus via the listen() below.
    //
    // After the updater resolves, we query `version_contract_state` to
    // discover the resolver's decision and gate the Tauri auto-updater
    // modal on it. Per the spec, at most one update affordance per launch:
    //   - decision == 'installer_required' → run checkForUpdates() to open
    //     the existing Tauri-updater modal (no 3 s wait — we already know).
    //   - decision in {hot_swap, up_to_date, fallback} → never call
    //     checkForUpdates() at launch. A long-interval (24 h) recheck
    //     covers users who keep the DAW open for days; that's set up below.
    ;(async () => {
      let unlisten: (() => void) | null = null
      try {
        const { listen } = await import('@tauri-apps/api/event')
        unlisten = await listen<{
          kind: string
          downloaded?: number
          total?: number
          version?: string
          reason?: string
          manifest_requires?: string
          running?: string
          target_version?: string | null
          track?: 'stable' | 'beta'
          release_url?: string | null
        }>('frontend-update-status', (e) => {
          const p = e.payload
          switch (p.kind) {
            case 'checking':
              setSplashStatus('Checking for updates...'); break
            case 'downloading':
              if (typeof p.total === 'number' && p.total > 0) {
                setSplashStatus(`Downloading update — ${formatBytes(p.total)}`)
              } else {
                setSplashStatus('Downloading update...')
              }
              break
            case 'verifying':
              setSplashStatus('Verifying signature...'); break
            case 'applying':
              setSplashStatus('Applying update...'); break
            case 'ready':
            case 'hot_swap_ready':
              // Auto-restart kicks in ~700ms after this event from the
              // Rust side; the dedicated `restarting` event below paints
              // the actual user-facing copy. Keep this branch quiet so
              // the splash doesn't briefly say "ready — restart to apply"
              // before flipping to "Restarting…".
              break
            case 'restarting':
              setSplashStatus(`Updating to ${p.version ?? ''} — restarting…`)
              break
            case 'installer_required':
              // Splash status copy aligned with surface A in the mockup.
              setSplashStatus('Installer upgrade required')
              break
            case 'up_to_date':
            case 'incompatible':
            case 'skipped':
              setSplashStatus('Starting up...'); break
          }
        })

        // Kick off the updater. The Rust command always resolves Ok(()).
        await invoke('frontend_update_check_and_apply')
      } catch (e) {
        // If the import or invoke itself blows up (older binary missing
        // the command, etc.), don't block the splash.
        console.warn('[frontend updater] init failed:', e)
      } finally {
        if (unlisten) {
          try { unlisten() } catch { /* event listener already detached */ }
        }
        setFrontendUpdateReady(true)
      }

      // Resolver decision — drives whether we surface the Tauri-updater
      // modal at all this session. Tolerates older binaries that don't
      // expose the command yet (treated as 'fallback'). The Rust side
      // caches the LaunchPlan it built for the splash, so this call
      // returns the EXACT decision the splash already rendered — no
      // chance of a CDN replica flipping the manifest between the two
      // fetches and showing both UIs. We pass `force_refresh: false`
      // explicitly so the cache-first contract is named at the call
      // site rather than leaning on the Rust-side default.
      let decision: VersionContractDecision = 'fallback'
      let releaseUrl: string | null = null
      try {
        const state = await invoke<VersionContractState>('version_contract_state', {
          forceRefresh: false,
        })
        if (cancelled) return
        decision = state.decision
        releaseUrl = state.release_url ?? null
        if (releaseUrl) {
          setUpdateInfo(prev => ({ ...prev, releaseUrl }))
        }
        if (state.installer_target && decision === 'installer_required') {
          // Pre-fill the modal version label with the manifest's hint so
          // the dialog reads correctly the moment Tauri's check() opens
          // it — the real version still comes from the auto-updater feed.
          setUpdateInfo(prev => ({
            ...prev,
            version: state.installer_target ?? prev.version,
          }))
        }
      } catch (e) {
        console.warn('[version contract] state query failed, defaulting to fallback:', e)
      }

      if (cancelled) return
      if (decision === 'installer_required') {
        // Skip the legacy 3 s wait — the resolver has already told us a
        // binary upgrade is required.
        await checkForUpdates()
      }
      // No else-branch: hot_swap / up_to_date / fallback all stay silent
      // at launch. A 24 h recheck timer below catches manifest pivots in
      // long-running sessions.
    })()

    // Crash detection: if the previous session didn't clear the alive marker,
    // surface the newest auto-save as a recovery option.
    ;(async () => {
      try {
        const crashed = await invoke<boolean>('autosave_detect_crash')
        if (crashed) {
          const latest = await invoke<{ path: string; modified_unix: number } | null>('autosave_latest')
          if (latest) setCrashInfo(latest)
        }
        await invoke('autosave_mark_alive')
      } catch {}
    })()

    // Long-interval recheck — covers users who keep the DAW running for
    // days. Re-invokes `version_contract_state` with `force_refresh: true`
    // so the Rust side bypasses its cache, re-fetches the manifest, and
    // updates the cached plan. The launch-time call above stays cache-
    // first and locked to the splash decision; this is the explicit
    // pivot path. Replaces the legacy 3 s `setTimeout(checkForUpdates,
    // 3000)` which raced the splash gate.
    const RECHECK_MS = 24 * 60 * 60 * 1000
    const dailyRecheck = setInterval(async () => {
      try {
        const state = await invoke<VersionContractState>('version_contract_state', {
          forceRefresh: true,
        })
        if (state.decision === 'installer_required') {
          if (state.release_url) {
            setUpdateInfo(prev => ({ ...prev, releaseUrl: state.release_url ?? null }))
          }
          await checkForUpdates()
        }
      } catch {
        // Manifest unreachable mid-session — silently ignore. An older
        // binary that doesn't accept the force_refresh arg will treat it
        // as an unknown field and still return the cached plan (Tauri's
        // arg deserialiser is lenient); the worst case is the recheck
        // does nothing, which is the correct degraded behaviour.
      }
    }, RECHECK_MS)
    return () => {
      cancelled = true
      clearInterval(dailyRecheck)
      document.removeEventListener('dragenter', blockDefaultDrag, { capture: true })
      document.removeEventListener('dragover', blockDefaultDrag, { capture: true })
      document.removeEventListener('drop', blockDefaultDrag, { capture: true })
    }
  }, [])

  // Auto-save: cadence driven by autosavePrefsStore (FL File Settings →
  // Backup section). 'never' disables the timer entirely; every other
  // option pings autosave_save while the project is dirty.
  const autosaveFrequency = useAutosavePrefsStore((s) => s.frequency)
  useEffect(() => {
    const ENABLED = localStorage.getItem('hardwave.daw.autoSaveEnabled') !== 'false'
    if (!ENABLED) return
    const intervalMs = frequencyIntervalMs(autosaveFrequency)
    if (intervalMs <= 0) return // 'never' — no timer
    const id = setInterval(() => {
      const { dirty } = useProjectStore.getState()
      if (dirty) {
        invoke('autosave_save').catch(() => {})
      }
    }, intervalMs)
    return () => clearInterval(id)
  }, [autosaveFrequency])

  useEffect(() => {
    let cancelled = false
    invoke<boolean>('get_pdc_enabled')
      .then(v => { if (!cancelled) setPdcEnabled(v) })
      .catch(() => {})
    return () => { cancelled = true }
  }, [])

  // Warn before closing the window if there are unsaved changes.
  //
  // Two distinct close paths and both must prompt:
  //
  //  1. `beforeunload` — fires when the WebView navigates away or the
  //     dev-server reloads. Tauri exposes the standard browser event.
  //  2. `onCloseRequested` — fires when the user hits the native window
  //     close button (X) or the platform OS sends close. The browser
  //     `beforeunload` never runs for native close because Tauri owns
  //     the chrome, so this listener is the only safety net there.
  //
  // For the native path we use Tauri's confirm dialog (modal native
  // popup) so the user gets a Save / Discard / Cancel choice; the
  // beforeunload version is intentionally lighter — the browser's
  // built-in unload prompt is the best we can do there.
  useEffect(() => {
    const handler = (e: BeforeUnloadEvent) => {
      if (useProjectStore.getState().dirty) {
        e.preventDefault()
        e.returnValue = ''
      }
    }
    window.addEventListener('beforeunload', handler)

    // Wire Tauri close-requested. Uses the same in-app
    // SaveChangesDialog as every other save-prompt path so the user
    // sees a Hardwave-styled cancel / discard / save dialog instead
    // of the OS-native modal. The earlier native `ask` dialog also
    // turned out to be the most likely cause of the "X button does
    // nothing" report — when `ask` fails to surface (plugin-dialog
    // permission missing on older binaries, or a stale promise from
    // a hot-swap reload), the close pipeline stays locked forever.
    // Routing through React state + Promise instead removes that
    // dependency entirely.
    let cleanupTauri: (() => void) | undefined
    void (async () => {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window')
        const win = getCurrentWindow()
        const unlisten = await win.onCloseRequested(async (event) => {
          if (!useProjectStore.getState().dirty) return
          // Pause the native close until the user picks an action.
          event.preventDefault()
          // Use the same styled SaveChangesDialog flow as the
          // confirmDiscardIfDirty path. Cancel = abort the close.
          // Discard = close immediately. Save = persist then close.
          const choice = await new Promise<SaveChangesChoice>(resolve => {
            savePromptResolver.current = resolve
            setSavePromptAction('Close anyway')
          })
          setSavePromptAction(null)
          savePromptResolver.current = null
          if (choice === 'cancel') return
          if (choice === 'save') {
            try {
              await useProjectStore.getState().saveProject()
              if (useProjectStore.getState().dirty) {
                // Save fell over — keep the window open so the user
                // can recover and try again.
                return
              }
            } catch {
              return
            }
          }
          useProjectStore.setState({ dirty: false })
          await win.close()
        })
        cleanupTauri = unlisten
      } catch {
        // Not running under Tauri (browser preview) — silently skip.
      }
    })()

    return () => {
      window.removeEventListener('beforeunload', handler)
      cleanupTauri?.()
    }
  }, [])

  const checkForUpdates = async () => {
    try {
      const { check } = await import('@tauri-apps/plugin-updater')
      const update = await check()
      if (update?.available) {
        setUpdateInfo(prev => ({
          ...prev,
          available: true,
          version: update.version,
          changelog: update.body || '',
          date: update.date || null,
        }))
      }
    } catch {
      // Not in Tauri or no update available
    }
  }

  const handleUpdate = useCallback(async () => {
    try {
      const { check } = await import('@tauri-apps/plugin-updater')
      const { relaunch } = await import('@tauri-apps/plugin-process')
      const update = await check()
      if (!update?.available) return

      setUpdateInfo(prev => ({ ...prev, downloading: true, error: null }))

      await update.downloadAndInstall((event: any) => {
        if (event.event === 'Progress' && 'contentLength' in event.data) {
          const total = (event.data as { contentLength: number }).contentLength
          if (total > 0) {
            setUpdateInfo(prev => {
              const chunkLen = (event.data as { chunkLength?: number }).chunkLength || 0
              const newProgress = Math.min(100, Math.round(((prev.progress / 100) * total + chunkLen) / total * 100))
              return { ...prev, progress: newProgress }
            })
          }
        }
        if (event.event === 'Finished') {
          setUpdateInfo(prev => ({ ...prev, downloading: false, downloaded: true, progress: 100 }))
        }
      })

      await relaunch()
    } catch (err) {
      setUpdateInfo(prev => ({ ...prev, downloading: false, error: String(err) }))
    }
  }, [])

  const handleDismissUpdate = useCallback(() => {
    setUpdateInfo(prev => ({ ...prev, dismissed: true }))
  }, [])

  const [savePromptAction, setSavePromptAction] = useState<string | null>(null)
  const savePromptResolver = useRef<((c: SaveChangesChoice) => void) | null>(null)
  const [showTemplateDialog, setShowTemplateDialog] = useState(false)

  const confirmDiscardIfDirty = useCallback(async (action: string): Promise<boolean> => {
    const { dirty } = useProjectStore.getState()
    if (!dirty) return true
    const choice = await new Promise<SaveChangesChoice>(resolve => {
      savePromptResolver.current = resolve
      setSavePromptAction(action)
    })
    setSavePromptAction(null)
    savePromptResolver.current = null
    if (choice === 'cancel') return false
    if (choice === 'save') {
      try {
        await useProjectStore.getState().saveProject()
        if (useProjectStore.getState().dirty) return false
      } catch (err) {
        await showErrorDialog('Save failed', String(err))
        return false
      }
    }
    return true
  }, [])

  const showErrorDialog = useCallback(async (title: string, msg: string) => {
    try {
      const { message } = await import('@tauri-apps/plugin-dialog')
      await message(msg, { title, kind: 'error' })
    } catch {
      window.alert(`${title}\n\n${msg}`)
    }
  }, [])

  const applyTemplate = useCallback(async (id: TemplateId) => {
    const ts = useTrackStore.getState()
    if (id === 'blank') return
    if (id === 'beat4') {
      await ts.addAudioTrack('Kick')
      await ts.addAudioTrack('Snare')
      await ts.addAudioTrack('Hi-Hat')
      await ts.addAudioTrack('Bass')
    } else if (id === 'vocal') {
      await ts.addAudioTrack('Vocal')
      await ts.addAudioTrack('Backing Vocals')
      await ts.addAudioTrack('FX Return')
    } else if (id === 'mixing') {
      const ps = usePluginStore.getState()
      const existingBefore = useTrackStore.getState().tracks.filter(t => t.kind !== 'Master')
      for (let i = 1; i <= 8; i++) await ts.addAudioTrack(`Track ${i}`)
      await ts.addAudioTrack('Bus A')
      await ts.addAudioTrack('Bus B')
      // Add EQ + Compressor to each of the 8 new audio tracks (Bus A/B stay clean).
      await ts.fetchTracks()
      const afterTracks = useTrackStore.getState().tracks.filter(t => t.kind !== 'Master')
      const newTracks = afterTracks.slice(existingBefore.length, existingBefore.length + 8)
      for (const tr of newTracks) {
        try {
          await ps.addToTrack(tr.id, 'hardwave.native.eq')
          await ps.addToTrack(tr.id, 'hardwave.native.compressor')
        } catch (err) {
          console.warn('Failed to insert native EQ+Comp on template track', tr.id, err)
        }
      }
    } else if (id.startsWith('user:')) {
      const tpl = useUserTemplateStore.getState().get(id.slice(5))
      if (!tpl) return
      await ts.fetchTracks()
      const existing = useTrackStore.getState().tracks.filter(t => t.kind !== 'Master')
      for (let i = 0; i < tpl.tracks.length; i++) {
        const def = tpl.tracks[i]
        if (def.kind === 'Midi') {
          await ts.addMidiTrack(def.name)
        } else {
          await ts.addAudioTrack(def.name)
        }
      }
      await ts.fetchTracks()
      const after = useTrackStore.getState().tracks.filter(t => t.kind !== 'Master')
      const created = after.slice(existing.length)
      for (let i = 0; i < created.length && i < tpl.tracks.length; i++) {
        const def = tpl.tracks[i]
        const tr = created[i]
        if (tr.color !== def.color) await ts.setTrackColor(tr.id, def.color)
        if (tr.volume_db !== def.volume_db) await ts.setVolume(tr.id, def.volume_db)
        if (tr.pan !== def.pan) await ts.setPan(tr.id, def.pan)
      }
      if (tpl.bpm && tpl.bpm > 0) {
        await useTransportStore.getState().setBpm(tpl.bpm)
      }
    }
  }, [])

  const handleSaveAsTemplate = useCallback(async () => {
    const name = window.prompt('Template name:')
    if (name === null) return
    const trimmed = name.trim()
    if (!trimmed) return
    const tracks = useTrackStore.getState().tracks
      .filter(t => t.kind !== 'Master')
      .map(t => ({
        name: t.name,
        kind: t.kind,
        color: t.color,
        volume_db: t.volume_db,
        pan: t.pan,
      }))
    if (tracks.length === 0) {
      window.alert('No tracks to save. Add some tracks first.')
      return
    }
    const bpm = useTransportStore.getState().bpm
    useUserTemplateStore.getState().add(trimmed, tracks, bpm)
  }, [])

  const handleNewProject = useCallback(async () => {
    if (!(await confirmDiscardIfDirty('Save changes before creating a new project'))) return
    setShowTemplateDialog(true)
  }, [confirmDiscardIfDirty])

  const handlePickTemplate = useCallback(async (id: TemplateId) => {
    setShowTemplateDialog(false)
    await newProject()
    await fetchTracks()
    await applyTemplate(id)
    await fetchTracks()
  }, [newProject, fetchTracks, applyTemplate])

  const warnIfMissingPlugins = useCallback(async () => {
    try {
      const missing = await invoke<Array<{ pluginId: string; trackName: string; slotIndex: number }>>('find_missing_plugins')
      if (missing.length === 0) return
      const byPlugin = new Map<string, Array<{ trackName: string; slotIndex: number }>>()
      for (const m of missing) {
        const arr = byPlugin.get(m.pluginId) ?? []
        arr.push({ trackName: m.trackName, slotIndex: m.slotIndex })
        byPlugin.set(m.pluginId, arr)
      }
      const lines: string[] = []
      for (const [pid, uses] of byPlugin) {
        const where = uses.map(u => `${u.trackName} · slot #${u.slotIndex + 1}`).join(', ')
        lines.push(`• ${pid}\n   ${where}`)
      }
      await showErrorDialog(
        'Missing plugins',
        `This project references ${missing.length} plugin instance${missing.length === 1 ? '' : 's'} that aren't installed or scanned. Their state is preserved — rescan or install the plugins to re-enable them.\n\n${lines.join('\n')}`,
      )
    } catch {}
  }, [showErrorDialog])

  const handleOpenProject = useCallback(async () => {
    if (!(await confirmDiscardIfDirty('Save changes before opening another project'))) return
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const selected = await open({
        filters: [{ name: 'Hardwave Project', extensions: ['hwp'] }],
        multiple: false,
      })
      if (!selected) return
      const path = selected as string
      try {
        await loadProject(path)
        await fetchTracks()
        await warnIfMissingPlugins()
      } catch (err) {
        useProjectStore.getState().removeRecent(path)
        await showErrorDialog('Could not open project', `${path}\n\n${err}`)
      }
    } catch {}
  }, [loadProject, fetchTracks, confirmDiscardIfDirty, showErrorDialog, warnIfMissingPlugins])

  const handleSaveProjectAs = useCallback(async () => {
    try {
      await saveProject(undefined)
      await invoke('autosave_clear').catch(() => {})
    } catch (err) {
      await showErrorDialog('Save failed', String(err))
    }
  }, [saveProject, showErrorDialog])

  const handleSaveProject = useCallback(async () => {
    try {
      await saveProject()
      await invoke('autosave_clear').catch(() => {})
    } catch (err) {
      await showErrorDialog('Save failed', String(err))
    }
  }, [saveProject, showErrorDialog])

  // Save the current project under an auto-suffixed name. FL Studio's
  // "Save new version" — picks the next free MyProject_N.hwd in the
  // same directory and saves there without prompting. Useful for
  // version-stamping after a big change.
  const handleSaveNewVersion = useCallback(async () => {
    const current = useProjectStore.getState().filePath
    if (!current) {
      // Not yet saved — fall through to Save As so the user names it.
      await handleSaveProjectAs()
      return
    }
    // Strip extension + trailing _N. e.g. "Track_3.hwd" → base "Track" + ext ".hwd"
    const match = current.match(/^(.*?)(?:_(\d+))?(\.[^.]+)?$/)
    const base = match?.[1] ?? current
    const ext = match?.[3] ?? '.hwd'
    let n = 2
    if (match?.[2]) n = parseInt(match[2], 10) + 1
    const candidate = `${base}_${n}${ext}`
    try {
      await saveProject(candidate)
      await invoke('autosave_clear').catch(() => {})
    } catch (err) {
      await showErrorDialog('Save new version failed', String(err))
    }
  }, [saveProject, handleSaveProjectAs, showErrorDialog])

  // Restore the last autosave snapshot. FL Studio's "Revert to last
  // backup" — picks up the most recent autosave_latest path and
  // loads it as if the user opened it manually. Fails silently when
  // there's nothing to restore.
  const handleRevertToBackup = useCallback(async () => {
    try {
      const latest = await invoke<{ path: string; modified_unix: number } | null>('autosave_latest')
      if (!latest) {
        await showErrorDialog('No backup found', 'No autosave snapshot is available for this session.')
        return
      }
      await loadProject(latest.path)
    } catch (err) {
      await showErrorDialog('Revert failed', String(err))
    }
  }, [loadProject, showErrorDialog])

  const handleCrashChoice = useCallback(async (c: CrashChoice) => {
    const info = crashInfo
    setCrashInfo(null)
    if (!info) return
    if (c === 'discard') {
      await invoke('autosave_clear').catch(() => {})
      return
    }
    if (c === 'recover') {
      try {
        await loadProject(info.path)
        await fetchTracks()
        useProjectStore.getState().markDirty()
        await warnIfMissingPlugins()
      } catch (err) {
        await showErrorDialog('Recovery failed', String(err))
      }
      return
    }
    if (c === 'open') {
      try {
        const { open } = await import('@tauri-apps/plugin-dialog')
        const selected = await open({
          filters: [{ name: 'Hardwave Project', extensions: ['hwp'] }],
          multiple: false,
        })
        if (selected) {
          await loadProject(selected as string)
          await fetchTracks()
          await warnIfMissingPlugins()
        }
      } catch {}
    }
  }, [crashInfo, loadProject, fetchTracks, showErrorDialog, warnIfMissingPlugins])

  const pasteAtPlayhead = useCallback(() => {
    const transport = useTransportStore.getState()
    const tracks = useTrackStore.getState()
    const sr = transport.sampleRate || 48000
    const playheadTicks = Math.round((transport.positionSamples / sr) * (transport.bpm / 60) * 960)
    const pasteAt = transport.editCursorTicks != null ? transport.editCursorTicks : playheadTicks
    tracks.pasteClipsAtPosition(pasteAt)
  }, [])

  const duplicateSelection = useCallback(() => {
    const tracks = useTrackStore.getState()
    const sel = tracks.selectedClipId
    if (!sel) return
    const t = tracks.tracks.find(tr => tr.clips.some(c => c.id === sel))
    if (t) tracks.duplicateClip(t.id, sel)
  }, [])

  const cutSelection = useCallback(async () => {
    const tracks = useTrackStore.getState()
    tracks.copySelectedClips()
    await tracks.deleteSelectedClips()
  }, [])

  const handleOpenRecent = useCallback(async (path: string) => {
    if (!(await confirmDiscardIfDirty('Save changes before opening this recent project'))) return
    try {
      await loadProject(path)
      await fetchTracks()
      await warnIfMissingPlugins()
    } catch (err) {
      useProjectStore.getState().removeRecent(path)
      await showErrorDialog('Could not open project', `${path}\n\nRemoved from recent projects.\n\n${err}`)
    }
  }, [loadProject, fetchTracks, confirmDiscardIfDirty, showErrorDialog, warnIfMissingPlugins])

  const handleExportAudio = useCallback(() => {
    setShowExport(true)
  }, [])

  const handleExportComplete = useCallback(async (result: { path: string; duration_secs: number }) => {
    setShowExport(false)
    const notif = useNotificationStore.getState()
    const dur = result.duration_secs.toFixed(1)
    notif.push('info', `Exported ${dur}s to WAV`, { detail: result.path })
    try {
      const { revealItemInDir } = await import('@tauri-apps/plugin-opener')
      await revealItemInDir(result.path)
    } catch {}
  }, [])

  const handleExportError = useCallback(async (msg: string) => {
    setShowExport(false)
    try {
      const { message } = await import('@tauri-apps/plugin-dialog')
      await message(`Export failed: ${msg}`, { title: 'Export audio', kind: 'error' })
    } catch {}
  }, [])

  const handleAddAutomationTrack = useCallback(async () => {
    await useTrackStore.getState().addAutomationTrack()
  }, [])

  const applyTrackTemplate = useCallback(async (templateId: string) => {
    const template = useTrackTemplateStore.getState().get(templateId)
    if (!template) return
    const tracks = useTrackStore.getState()
    const before = new Set(tracks.tracks.map(t => t.id))
    if (template.kind === 'Midi') await tracks.addMidiTrack(template.trackName)
    else await tracks.addAudioTrack(template.trackName)
    const after = useTrackStore.getState().tracks
    const created = after.find(t => !before.has(t.id))
    if (!created) return
    await useTrackStore.getState().setTrackColor(created.id, template.color)
    await useTrackStore.getState().setVolume(created.id, template.volumeDb)
    await useTrackStore.getState().setPan(created.id, template.pan)
    useNotificationStore.getState().push('info', `Added track from template "${template.name}"`)
  }, [])

  // Global keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return

      const transport = useTransportStore.getState()
      const tracks = useTrackStore.getState()

      // Dev-only shortcut — hardcoded, not user-rebindable.
      if (e.code === 'KeyD' && e.ctrlKey && e.shiftKey) {
        e.preventDefault()
        setShowDevPanel(v => !v)
        return
      }

      // Hard close shortcut — Cmd+Q (mac) / Ctrl+Q (win/linux).
      // Routes through `destroy()` instead of `close()` so any stuck
      // CloseRequested handler can't block it. Provides a reliable
      // panic-quit when the X button has gone unresponsive.
      if (e.code === 'KeyQ' && (e.ctrlKey || e.metaKey)) {
        e.preventDefault()
        void (async () => {
          try {
            const { getCurrentWindow } = await import('@tauri-apps/api/window')
            await getCurrentWindow().destroy()
          } catch {}
        })()
        return
      }

      // Multimedia keyboard keys (FL Studio convention). These come
      // through e.code as MediaPlayPause / MediaStop / MediaTrackNext /
      // MediaTrackPrevious. Mapped to transport so the user's playback
      // keyboard / Mac Touch Bar works without rebinding anything.
      if (e.code === 'MediaPlayPause') {
        e.preventDefault()
        transport.togglePlayback()
        return
      }
      if (e.code === 'MediaStop') {
        e.preventDefault()
        transport.stop()
        return
      }
      if (e.code === 'MediaTrackNext' || e.code === 'MediaTrackPrevious') {
        // FL maps FF/RW to "jump to next/previous time marker".
        e.preventDefault()
        const sr = transport.sampleRate || 48000
        const PPQ = 960
        const playheadTicks = Math.round(
          (transport.positionSamples / sr) * (transport.bpm / 60) * PPQ,
        )
        const ms = useMarkerStore.getState()
        const target =
          e.code === 'MediaTrackNext'
            ? ms.jumpToNext(playheadTicks)
            : ms.jumpToPrev(playheadTicks)
        if (target) {
          const secs = target.tick / PPQ / (transport.bpm / 60)
          transport.setPosition(Math.round(secs * sr))
        }
        return
      }

      // F1 opens the in-app help overlay (Shift+F1 opens the shortcuts panel).
      if (e.code === 'F1') {
        e.preventDefault()
        if (e.shiftKey) setShowShortcuts(v => !v)
        else setShowHelp(v => !v)
        return
      }

      // Alt+F7 toggleTouchController is now routed through the
      // shortcutsStore action dispatcher so users can rebind it from
      // the shortcuts panel. The hard-coded fallback was removed when
      // the action entered DEFAULTS.

      // Capture mode swallows every keypress so it can bind a new shortcut.
      if (useShortcutsStore.getState().capturingFor) return

      const action = useShortcutsStore.getState().matchEvent(e)
      if (!action) return
      e.preventDefault()

      switch (action) {
        case 'newProject':  handleNewProject(); return
        case 'openProject': handleOpenProject(); return
        case 'save':        handleSaveProject(); return
        case 'saveAs':      handleSaveProjectAs(); return
        case 'selectAll':   tracks.selectAllClips(); return
        case 'duplicate':   duplicateSelection(); return
        case 'copy':        tracks.copySelectedClips(); return
        case 'cut':         cutSelection(); return
        case 'paste':       pasteAtPlayhead(); return
        case 'undo':        tracks.undo(); return
        case 'redo':        tracks.redo(); return
        case 'togglePlay':  transport.togglePlayback(); return
        case 'deleteSelection': tracks.deleteSelectedClips(); return
        case 'splitClip': {
          const sel = tracks.selectedClipId
          if (!sel) return
          const t = tracks.tracks.find(tr => tr.clips.some(c => c.id === sel))
          if (!t) return
          const sr = transport.sampleRate || 48000
          const playheadTicks = Math.round((transport.positionSamples / sr) * (transport.bpm / 60) * 960)
          const atTicks = transport.editCursorTicks != null ? transport.editCursorTicks : playheadTicks
          const clip = t.clips.find(c => c.id === sel)
          if (clip && atTicks > clip.position_ticks && atTicks < clip.position_ticks + clip.length_ticks) {
            tracks.splitClip(t.id, sel, atTicks)
          }
          return
        }
        case 'gotoStart': transport.setPosition(0); return
        case 'gotoEnd': {
          const allClips = tracks.tracks.flatMap(t => t.clips || [])
          if (allClips.length > 0) {
            const lastEnd = Math.max(...allClips.map(c => c.position_ticks + c.length_ticks))
            const sr = transport.sampleRate || 48000
            const samplesPerTick = (sr * 60) / (transport.bpm * 960)
            transport.setPosition(Math.round(lastEnd * samplesPerTick))
          }
          return
        }
        case 'toggleLoop':            transport.toggleLoop(); return
        case 'toggleRecord':          void transport.toggleRecording(); return
        case 'panicStop': {
          // Send NoteOff for every MIDI note (all-notes-off panic).
          // This mirrors FL Studio's Ctrl+H "Stop sound (panic)".
          // Hits every channel via the same inject_midi_event pipeline
          // that midir + typing keyboard + virtual keyboard use, so any
          // active voice in MidiTrackNode / plug-in chains releases.
          import('@tauri-apps/api/core').then(({ invoke }) => {
            for (let note = 0; note < 128; note++) {
              void invoke('inject_midi_event', {
                event: { kind: 'note_off', channel: 0, note },
              })
            }
            transport.stop()
          })
          return
        }
        case 'toggleMetronome': {
          import('./stores/metronomeStore').then(({ useMetronomeStore }) => {
            useMetronomeStore.getState().toggleEnabled()
          })
          return
        }
        case 'nextPattern': {
          import('./stores/patternStore').then(({ usePatternStore }) => {
            usePatternStore.getState().nextPattern()
          })
          return
        }
        case 'prevPattern': {
          import('./stores/patternStore').then(({ usePatternStore }) => {
            usePatternStore.getState().prevPattern()
          })
          return
        }
        case 'nextEmptyPattern': {
          import('./stores/patternStore').then(({ usePatternStore }) => {
            // No dedicated 'next empty pattern' API yet; addPattern
            // creates a fresh empty one and selects it, which matches
            // the spirit of FL Studio's F4 behaviour.
            usePatternStore.getState().addPattern()
          })
          return
        }
        case 'selectPattern1':
        case 'selectPattern2':
        case 'selectPattern3':
        case 'selectPattern4':
        case 'selectPattern5':
        case 'selectPattern6':
        case 'selectPattern7':
        case 'selectPattern8':
        case 'selectPattern9': {
          const idx = Number(action.replace('selectPattern', '')) - 1
          import('./stores/patternStore').then(({ usePatternStore }) => {
            const s = usePatternStore.getState()
            const target = s.patterns[idx]
            if (target) s.setActive(target.id)
          })
          return
        }
        case 'togglePlaylist':        setShowPlaylist(v => !v); return
        case 'toggleChannelRack':     setShowChannelRack(v => !v); return
        case 'togglePianoRoll':       setShowPianoRoll(v => !v); return
        case 'toggleBrowser':         setShowBrowser(v => !v); return
        case 'toggleMixer':           setShowMixer(v => !v); return
        case 'toggleShortcutsPanel':  setShowShortcuts(v => !v); return
        case 'toggleTouchController': toggleTouchController(); return
        case 'toggleTypingKeyboard': {
          useTypingKeyboardStore.getState().toggle()
          return
        }
        case 'toggleMidiSettings':    setShowAudioSettings(v => !v); return
        case 'toggleSongInfo':
          // F11 — FL Studio convention. ProjectInfoDialog renders the
          // metadata fields + auto-saves to the project on Save.
          setShowProjectInfo(v => !v)
          return
        case 'closeAllWindows': {
          // FL F12 — slam every transient panel shut at once.
          setShowBrowser(false); setShowMixer(false); setShowChannelRack(false)
          setShowPianoRoll(false); setShowShortcuts(false); setShowHelp(false)
          setShowAudioSettings(false); setShowThemePicker(false); setShowAbout(false)
          setShowRoadmap(false); setShowTrackTemplateManager(false)
          setShowLoudness(false); setShowOscilloscope(false); setShowSpectrum(false)
          setShowMidiMappings(false); setShowTempoMap(false)
          setTouchControllerVisible(false)
          return
        }
        case 'cycleWindows':
        case 'toggleMaxMinPlaylist':
        case 'renameSelected':
        case 'openToolSelector': {
          import('./stores/notificationStore').then(({ useNotificationStore }) => {
            useNotificationStore.getState().push('info', `Shortcut ${action} bound — feature wiring queued.`)
          })
          return
        }
        case 'toolDraw':              usePlaylistToolStore.getState().setTool('draw'); return
        case 'toolPaint':             usePlaylistToolStore.getState().setTool('paint'); return
        case 'toolSlice':             usePlaylistToolStore.getState().setTool('slice'); return
        case 'toolDelete':            usePlaylistToolStore.getState().setTool('delete'); return
        case 'toolMute':              usePlaylistToolStore.getState().setTool('mute'); return
        case 'toolSlip':              usePlaylistToolStore.getState().setTool('slip'); return
        case 'toolSelect':            usePlaylistToolStore.getState().setTool('select'); return
        case 'toolZoom':              usePlaylistToolStore.getState().setTool('zoom'); return
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [handleNewProject, handleOpenProject, handleSaveProject, handleSaveProjectAs, fetchTracks, duplicateSelection, cutSelection, pasteAtPlayhead])

  // Top-bar menus — built here because every action (new/open/save, undo/
  // redo, view toggles, dialog opens, view scale) lives in App's scope.
  // HwApp renders the dropdown chrome via <HwTopMenu menus={...} />. The
  // memo deps include only the values that actually change menu labels
  // (recent list, view toggles, pdc state) — all callbacks are stable.
  const menus: MenuDef[] = useMemo(() => {
    const recentItems: MenuItem[] = recentProjects.length === 0
      ? [{ label: '(none)', disabled: true }]
      : [
          ...recentProjects.slice(0, 10).map((p) => ({
            label: shortenRecentPath(p),
            action: () => handleOpenRecent(p),
          })),
          { separator: true, label: '' },
          { label: 'Clear recent', action: () => useProjectStore.getState().recentProjects.forEach(p => useProjectStore.getState().removeRecent(p)) },
        ]

    return [
      {
        label: 'File',
        items: [
          { label: 'New project', shortcut: 'Ctrl+N', action: handleNewProject },
          { label: 'Open project…', shortcut: 'Ctrl+O', action: handleOpenProject },
          { label: 'Recent projects', submenu: recentItems },
          { separator: true, label: '' },
          { label: 'Save', shortcut: 'Ctrl+S', action: handleSaveProject },
          { label: 'Save as…', shortcut: 'Ctrl+Shift+S', action: handleSaveProjectAs },
          { label: 'Save new version', action: handleSaveNewVersion },
          { label: 'Save as template…', action: handleSaveAsTemplate },
          { separator: true, label: '' },
          { label: 'Revert to last backup', action: handleRevertToBackup },
          { separator: true, label: '' },
          { label: 'Project info…', shortcut: 'F11', action: () => setShowProjectInfo(true) },
          { separator: true, label: '' },
          { label: 'Export audio…', action: handleExportAudio },
          { separator: true, label: '' },
          {
            label: 'Import audio file…',
            shortcut: 'Ctrl+I',
            action: async () => {
              const push = useNotificationStore.getState().push
              try {
                const { open } = await import('@tauri-apps/plugin-dialog')
                const selected = await open({
                  multiple: true,
                  filters: [{ name: 'Audio', extensions: ['wav', 'flac', 'mp3', 'ogg', 'aac', 'm4a'] }],
                })
                if (!selected) return
                const paths = Array.isArray(selected) ? selected : [selected]
                if (paths.length === 0) return

                const ts = useTrackStore.getState()
                const audio = ts.tracks.filter(t => t.kind === 'Audio')
                let trackId: string | null = ts.selectedTrackId
                if (!trackId || !audio.find(t => t.id === trackId)) {
                  if (audio.length === 0) {
                    trackId = await ts.addAudioTrack()
                  } else {
                    trackId = audio[0].id
                  }
                }
                if (!trackId) {
                  push('error', 'Could not find or create an audio track')
                  return
                }

                const track = useTrackStore.getState().tracks.find(t => t.id === trackId)
                let offsetTicks = 0
                if (track) {
                  for (const clip of track.clips) {
                    const end = clip.position_ticks + clip.length_ticks
                    if (end > offsetTicks) offsetTicks = end
                  }
                }

                let imported = 0
                for (const path of paths) {
                  try {
                    const result = await ts.importAudioFile(trackId, path, offsetTicks)
                    offsetTicks += result.length_ticks
                    imported++
                  } catch (err) {
                    console.error('Import failed:', path, err)
                  }
                }
                push('info', imported === paths.length
                  ? `Imported ${imported} file${imported === 1 ? '' : 's'}`
                  : `Imported ${imported} of ${paths.length} files — ${paths.length - imported} failed`)
              } catch (err) {
                push('error', 'Import failed', { detail: String(err) })
              }
            },
          },
          { separator: true, label: '' },
          { label: 'Exit', action: async () => {
            try {
              const { getCurrentWindow } = await import('@tauri-apps/api/window')
              await getCurrentWindow().close()
            } catch {}
          } },
        ],
      },
      {
        label: 'Edit',
        items: [
          { label: 'Undo', shortcut: 'Ctrl+Z', action: () => useTrackStore.getState().undo() },
          { label: 'Redo', shortcut: 'Ctrl+Y', action: () => useTrackStore.getState().redo() },
          { label: 'History…', action: () => setShowHistory(true) },
          { separator: true, label: '' },
          { label: 'Cut', shortcut: 'Ctrl+X', action: cutSelection },
          { label: 'Copy', shortcut: 'Ctrl+C', action: () => useTrackStore.getState().copySelectedClips() },
          { label: 'Paste', shortcut: 'Ctrl+V', action: pasteAtPlayhead },
          { label: 'Duplicate', shortcut: 'Ctrl+D', action: duplicateSelection },
          { separator: true, label: '' },
          { label: 'Select all', shortcut: 'Ctrl+A', action: () => useTrackStore.getState().selectAllClips() },
        ],
      },
      {
        label: 'Add',
        items: [
          { label: 'Audio track', action: () => useTrackStore.getState().addAudioTrack() },
          { label: 'Instrument track', action: () => useTrackStore.getState().addMidiTrack() },
          { label: 'Automation track', action: handleAddAutomationTrack },
          { separator: true, label: '' },
          {
            label: 'Rescan plug-ins',
            action: async () => {
              try {
                const plugs = await invoke<unknown[]>('scan_plugins')
                useNotificationStore.getState().push(
                  'info',
                  `Plug-in scan complete — ${plugs.length} discovered`,
                )
              } catch (err) {
                useNotificationStore.getState().push('error', `Plug-in scan failed: ${String(err)}`)
              }
            },
          },
          { separator: true, label: '' },
          { label: 'Manage track templates…', action: () => setShowTrackTemplateManager(true) },
        ],
      },
      {
        label: 'Patterns',
        items: [
          { label: 'New pattern', shortcut: 'F4', action: () => usePatternStore.getState().addPattern() },
          { label: 'Find first empty', action: () => usePatternStore.getState().findFirstEmpty() },
          { label: 'Insert one (after active)', action: () => usePatternStore.getState().insertAfterActive() },
          { separator: true, label: '' },
          { label: 'Clone pattern', action: () => usePatternStore.getState().clonePattern() },
          { label: 'Delete pattern', shortcut: 'Del', action: () => usePatternStore.getState().deletePattern() },
          { separator: true, label: '' },
          { label: 'Move up', action: () => usePatternStore.getState().moveActiveUp() },
          { label: 'Move down', action: () => usePatternStore.getState().moveActiveDown() },
          { separator: true, label: '' },
          { label: 'Random color', action: () => usePatternStore.getState().randomColorActive() },
        ],
      },
      {
        label: 'View',
        items: [
          { label: `${showBrowser ? '✓ ' : '   '}Browser`, shortcut: 'Alt+F8', action: () => setShowBrowser(v => !v) },
          { label: `${showPlaylist ? '✓ ' : '   '}Playlist`, shortcut: 'F5', action: () => setShowPlaylist(v => !v) },
          { label: `${showChannelRack ? '✓ ' : '   '}Channel Rack`, shortcut: 'F6', action: () => setShowChannelRack(v => !v) },
          { label: `${showPianoRoll ? '✓ ' : '   '}Piano Roll`, shortcut: 'F7', action: () => setShowPianoRoll(v => !v) },
          { label: `${showMixer ? '✓ ' : '   '}Mixer`, shortcut: 'F9', action: () => setShowMixer(v => !v) },
          { separator: true, label: '' },
          { label: 'Tempo tapper…', action: () => setShowTempoTapper(true) },
        ],
      },
      {
        label: 'Options',
        items: [
          // System settings — FL Options menu top group
          { label: 'MIDI settings…', shortcut: 'F10', action: () => setShowAudioSettings(true) },
          { label: 'Audio settings…', action: () => setShowAudioSettings(true) },
          { label: 'General settings…', action: () => setShowAudioSettings(true) },
          { label: 'File settings…', action: () => setShowAudioSettings(true) },
          { label: 'Theme settings…', action: () => setShowThemePicker(true) },
          { separator: true, label: '' },
          // Project settings
          { label: 'Project info…', shortcut: 'F11', action: () => setShowProjectInfo(true) },
          { label: 'Tempo map…', action: () => setShowTempoMap(true) },
          { separator: true, label: '' },
          // Switches — togglable runtime state mirrors FL's Switches section
          { label: `${typingKeyboardEnabled ? '✓ ' : '   '}Typing keyboard to piano`, shortcut: 'Ctrl+T', action: () => useTypingKeyboardStore.getState().toggle() },
          { label: `${metronomeEnabled ? '✓ ' : '   '}Metronome`, shortcut: 'Ctrl+M', action: () => useMetronomeStore.getState().toggleEnabled() },
          { label: `${pdcEnabled ? '✓ ' : '   '}Plugin delay compensation`, action: () => setPdcEnabled(v => !v) },
          { label: `${useNewMixer ? '✓ ' : '   '}Experimental — FL Wide 2 mixer`, action: () => setUseNewMixer(!useNewMixer) },
          { separator: true, label: '' },
          // UI
          {
            label: 'UI scale',
            submenu: [
              {
                label: `${uiScaleMode === 'auto' ? '✓ ' : '   '}Auto (detected: ${uiScaleEffective}%)`,
                action: () => setUiScaleMode('auto'),
              },
              { separator: true, label: '' },
              ...UI_SCALE_OPTIONS.map((scale): MenuItem => ({
                label: `${uiScaleMode === scale ? '✓ ' : '   '}${scale}%`,
                action: () => setUiScaleMode(scale as UiScale),
              })),
            ],
          },
        ],
      },
      {
        label: 'Tools',
        items: [
          // Analyzers
          { label: 'Loudness meter…', action: () => setShowLoudness(true) },
          { label: 'Oscilloscope…', action: () => setShowOscilloscope(true) },
          { label: 'Spectrum analyzer…', action: () => setShowSpectrum(true) },
          { separator: true, label: '' },
          // MIDI / controllers
          { label: 'MIDI mappings…', action: () => setShowMidiMappings(true) },
          { label: 'Touch Controller', shortcut: 'Alt+F7', action: () => toggleTouchController() },
          {
            label: typingKeyboardEnabled
              ? 'Typing keyboard: On'
              : 'Typing keyboard: Off',
            action: () => useTypingKeyboardStore.getState().toggle(),
          },
          { separator: true, label: '' },
          // Macros — FL Tools menu parity
          {
            label: 'Macros',
            submenu: [
              {
                label: 'Panic — stop all sound',
                shortcut: 'Ctrl+H',
                action: () => {
                  for (let note = 0; note < 128; note++) {
                    void invoke('inject_midi_event', { event: { kind: 'note_off', channel: 0, note } })
                  }
                  void invoke('stop')
                },
              },
              {
                label: 'Cancel recording',
                action: async () => {
                  await invoke('cancel_recording').catch(() => {})
                  useTransportStore.setState({ recording: false, recordStartSample: null })
                },
              },
            ],
          },
        ],
      },
      {
        label: 'Help',
        items: [
          { label: 'Help topics', shortcut: 'F1', action: () => setShowHelp(v => !v) },
          { label: 'Keyboard shortcuts', shortcut: 'Shift+F1', action: () => setShowShortcuts(v => !v) },
          { label: 'Roadmap', action: () => setShowRoadmap(v => !v) },
          { label: 'Re-run MIDI setup wizard…', action: () => useSetupWizardStore.getState().open() },
          { separator: true, label: '' },
          { label: 'Online user manual', action: () => window.open('https://github.com/Dishairano/hardwave-daw/wiki', '_blank', 'noopener,noreferrer') },
          { label: 'Release notes', action: () => window.open('https://github.com/Dishairano/hardwave-daw/releases', '_blank', 'noopener,noreferrer') },
          { label: 'Report an issue', action: () => window.open('https://github.com/Dishairano/hardwave-daw/issues', '_blank', 'noopener,noreferrer') },
          { separator: true, label: '' },
          {
            label: 'Check for updates…',
            action: async () => {
              const push = useNotificationStore.getState().push
              try {
                const { check } = await import('@tauri-apps/plugin-updater')
                const { getVersion } = await import('@tauri-apps/api/app')
                const [update, currentVersion] = await Promise.all([check(), getVersion()])
                if (update?.available) {
                  setUpdateInfo(prev => ({
                    ...prev,
                    available: true,
                    dismissed: false,
                    version: update.version,
                    changelog: update.body || '',
                    date: update.date || null,
                  }))
                } else {
                  push('info', `You're up to date — running v${currentVersion}`)
                }
              } catch (err) {
                push('error', 'Update check failed', { detail: String(err) })
              }
            },
          },
          { separator: true, label: '' },
          { label: 'About Hardwave DAW', action: () => setShowAbout(true) },
        ],
      },
    ]
  }, [
    recentProjects, showBrowser, showPlaylist, showChannelRack, showPianoRoll, showMixer, pdcEnabled,
    uiScaleMode, uiScaleEffective, setUiScaleMode,
    handleNewProject, handleOpenProject, handleSaveProject, handleSaveProjectAs, handleSaveAsTemplate,
    handleExportAudio, handleOpenRecent, handleAddAutomationTrack, cutSelection, pasteAtPlayhead, duplicateSelection,
  ])

  return (
    <div style={{
      display: 'flex',
      flexDirection: 'column',
      height: '100vh',
      width: '100vw',
      background: hw.bg,
    }}>
      {showSplash && (
        <SplashScreen
          dataReady={dataReady}
          statusText={splashStatus}
          onFinished={() => setShowSplash(false)}
        />
      )}

      <HwApp
        showBrowser={showBrowser}
        showPlaylist={showPlaylist}
        showChannelRack={showChannelRack}
        showPianoRoll={showPianoRoll}
        showMixer={showMixer}
        isMobile={isMobile}
        mobilePanel={mobilePanel}
        menus={menus}
        onToggleBrowser={() => setShowBrowser(v => !v)}
        onToggleChannelRack={() => setShowChannelRack(v => !v)}
        onTogglePianoRoll={() => setShowPianoRoll(v => !v)}
        onToggleMixer={() => setShowMixer(v => !v)}
        onTogglePlaylist={() => setShowPlaylist(v => !v)}
        onOpenTempoTapper={() => setShowTempoTapper(true)}
        onAction={(id) => {
          // Mirror the keyboard-shortcut dispatch path so toolbar
          // buttons fire the same handlers. Only the actions the
          // toolbar surfaces are routed; other ActionIds keep
          // flowing through the existing keydown switch. `tracks`
          // is scoped to the keydown effect, so read store state
          // directly here for Copy.
          switch (id) {
            case 'save':       handleSaveProject(); return
            case 'saveAs':     handleSaveProjectAs(); return
            case 'cut':        cutSelection(); return
            case 'copy':       useTrackStore.getState().copySelectedClips(); return
            case 'paste':      pasteAtPlayhead(); return
            case 'duplicate':  duplicateSelection(); return
          }
        }}
        onOpenExport={() => setShowExport(true)}
      />

      {isMobile && (
        <MobileTabBar
          active={mobilePanel}
          onSelect={selectMobilePanel}
        />
      )}

      <FloatingPanels
        showBrowser={showBrowser}
        showPlaylist={showPlaylist}
        showChannelRack={showChannelRack}
        showPianoRoll={showPianoRoll}
        showMixer={showMixer}
        onHideBrowser={() => setShowBrowser(false)}
        onHidePlaylist={() => setShowPlaylist(false)}
        onHideChannelRack={() => setShowChannelRack(false)}
        onHidePianoRoll={() => setShowPianoRoll(false)}
        onHideMixer={() => setShowMixer(false)}
      />

      {/* Floating detached panels */}
      {showRoadmap && <Roadmap onClose={() => setShowRoadmap(false)} />}
      {showAudioSettings && <AudioSettings onClose={() => setShowAudioSettings(false)} />}
      {showThemePicker && <ThemePicker onClose={() => setShowThemePicker(false)} />}
      <ColorPicker />

      {showAbout && <AboutDialog onClose={() => setShowAbout(false)} />}
      <ShortcutsPanel open={showShortcuts} onClose={() => setShowShortcuts(false)} />
      <HelpOverlay open={showHelp} onClose={() => setShowHelp(false)} />
      <TrackTemplateManager open={showTrackTemplateManager} onClose={() => setShowTrackTemplateManager(false)} />
      {sampleEditorPath && <SampleEditor path={sampleEditorPath} onClose={closeSampleEditor} />}
      {beatSlicerPath && <BeatSlicer path={beatSlicerPath} onClose={closeBeatSlicer} />}
      {showLoudness && <LoudnessMeter onClose={() => setShowLoudness(false)} />}
      {showOscilloscope && <Oscilloscope onClose={() => setShowOscilloscope(false)} />}
      {showSpectrum && <SpectrumAnalyzer onClose={() => setShowSpectrum(false)} />}
      <VirtualKeyboard visible={touchControllerVisible} onClose={() => setTouchControllerVisible(false)} />
      <SetupWizard />
      {showProjectInfo && <ProjectInfoDialog onClose={() => setShowProjectInfo(false)} />}
      {showTempoTapper && <TempoTapper onClose={() => setShowTempoTapper(false)} />}
      {showMidiMappings && (
        <MidiMappingsPanel
          onClose={() => { setShowMidiMappings(false); setMidiLearnPreset(undefined) }}
          initialTarget={midiLearnPreset}
        />
      )}

      {showTempoMap && <TempoMapDialog onClose={() => setShowTempoMap(false)} />}
      {showHistory && <HistoryPanel onClose={() => setShowHistory(false)} />}
      <PrecountOverlay />
      {showDevPanel && <DevPanel onClose={() => setShowDevPanel(false)} />}

      {savePromptAction && (
        <SaveChangesDialog
          action={savePromptAction}
          onChoice={(c) => savePromptResolver.current?.(c)}
        />
      )}

      {showTemplateDialog && (
        <TemplateDialog
          onPick={handlePickTemplate}
          onCancel={() => setShowTemplateDialog(false)}
        />
      )}

      {showExport && (
        <ExportDialog
          initial={{
            bitDepth: 0,
            sampleRate: 48000,
            tailSecs: 2.0,
            defaultName: useProjectStore.getState().projectName || 'Untitled',
          }}
          onCancel={() => setShowExport(false)}
          onComplete={handleExportComplete}
          onError={handleExportError}
        />
      )}

      {showWelcome && !showSplash && !crashInfo && (
        <WelcomeScreen
          recentProjects={recentProjects}
          onOpenRecent={handleOpenRecent}
          onNewProject={handleNewProject}
          onOpenProject={handleOpenProject}
          onOpenSampleProject={async () => {
            await newProject()
            await applyTemplate('beat4')
          }}
          onOpenAudioSettings={() => setShowAudioSettings(true)}
          onDismiss={dismissWelcome}
        />
      )}

      {crashInfo && !showSplash && (
        <CrashRecoveryDialog
          autosavePath={crashInfo.path}
          modifiedUnix={crashInfo.modified_unix}
          onChoice={handleCrashChoice}
        />
      )}

      <NotificationHost />
      <MetronomeScheduler />

      {/* Update modal — same pattern as Hardwave Suite */}
      {updateInfo.available && !updateInfo.dismissed && (
        <UpdateModal
          version={updateInfo.version}
          changelog={updateInfo.changelog}
          date={updateInfo.date}
          downloading={updateInfo.downloading}
          progress={updateInfo.progress}
          downloaded={updateInfo.downloaded}
          error={updateInfo.error}
          releaseUrl={updateInfo.releaseUrl ?? null}
          onUpdate={handleUpdate}
          onDismiss={handleDismissUpdate}
        />
      )}
    </div>
  )
}

function MainLayout({
  showBrowser, showPlaylist, showChannelRack, showPianoRoll, showMixer,
  isMobile, mobilePanel,
}: {
  showBrowser: boolean; showPlaylist: boolean;
  showChannelRack: boolean; showPianoRoll: boolean; showMixer: boolean;
  isMobile: boolean; mobilePanel: MobilePanel;
}) {
  const layout = usePanelLayoutStore(s => s.layout)
  const [playlistHint, setPlaylistHint] = useState('')

  // Phone mode: show exactly one panel, full-width, no side dock.
  if (isMobile) {
    return (
      <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 0 }}>
        {mobilePanel === 'browser' && (
          <div data-testid="panel-browser" style={{ flex: 1, display: 'flex', overflow: 'auto', WebkitOverflowScrolling: 'touch' }}>
            <Browser />
          </div>
        )}
        {mobilePanel === 'channelRack' && (
          <div data-testid="panel-channel-rack" style={{ flex: 1, minHeight: 0, overflow: 'auto', WebkitOverflowScrolling: 'touch' }}>
            <ChannelRack />
          </div>
        )}
        {mobilePanel === 'pianoRoll' && (
          <div data-testid="panel-piano-roll" style={{ flex: 1, minHeight: 0, overflow: 'auto', WebkitOverflowScrolling: 'touch' }}>
            <PianoRoll />
          </div>
        )}
        {mobilePanel === 'playlist' && (
          <div data-testid="panel-playlist" style={{ flex: 1, display: 'flex', overflow: 'auto', WebkitOverflowScrolling: 'touch', minHeight: 0 }}>
            <TrackList />
            <Arrangement />
          </div>
        )}
        {mobilePanel === 'mixer' && (
          <div data-testid="panel-mixer" style={{ flex: 1, minHeight: 0, overflow: 'auto', WebkitOverflowScrolling: 'touch' }}>
            <MixerPanel />
          </div>
        )}
      </div>
    )
  }

  const browserDocked = showBrowser && !layout.browser.floating
  const channelRackDocked = showChannelRack && !layout.channelRack.floating
  const pianoRollDocked = showPianoRoll && !layout.pianoRoll.floating
  const mixerDocked = showMixer && !layout.mixer.floating
  const playlistDocked = showPlaylist && !layout.playlist.floating
  return (
    <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
      {browserDocked && <div data-testid="panel-browser" style={{ width: 240, flexShrink: 0, display: 'flex' }}><Browser /></div>}

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {channelRackDocked && (
          <div data-testid="panel-channel-rack" style={{
            flex: playlistDocked ? undefined : 1,
            height: playlistDocked ? '55%' : undefined,
            minHeight: 120,
            borderBottom: playlistDocked ? `1px solid ${hw.borderDark}` : undefined,
          }}>
            <ChannelRack />
          </div>
        )}

        {pianoRollDocked && (
          <div data-testid="panel-piano-roll" style={{
            flex: 1, minHeight: 200,
            borderBottom: playlistDocked ? `1px solid ${hw.borderDark}` : undefined,
          }}>
            <PianoRoll />
          </div>
        )}

        {playlistDocked && (
          <div data-testid="panel-playlist" style={{
            flex: 1, display: 'flex', flexDirection: 'column',
            overflow: 'hidden', minHeight: 80,
            background: '#000',
            borderTop: `1px solid ${hw.borderLight}`,
            position: 'relative',
          }}>
            {/* Hardwave panel signature: 2 px red-gradient top stripe */}
            <div style={{
              position: 'absolute', top: 0, left: 0, right: 0, height: 2,
              background: `linear-gradient(90deg, ${hw.secondary}, ${hw.accentLight}, ${hw.secondary})`,
              zIndex: 2, pointerEvents: 'none',
            }} />
            <PlaylistHeader />
            <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 0 }}>
              <TrackList />
              <Arrangement onSetHint={setPlaylistHint} />
            </div>
            <PlaylistHintBar text={playlistHint} />
          </div>
        )}

        {mixerDocked && (
          <div data-testid="panel-mixer" style={{
            height: (showPlaylist || channelRackDocked || pianoRollDocked) ? 220 : 'auto',
            flex: (showPlaylist || channelRackDocked || pianoRollDocked) ? undefined : 1,
            borderTop: `1px solid ${hw.borderDark}`,
          }}>
            <MixerPanel />
          </div>
        )}
      </div>
    </div>
  )
}

// Mockup-style playlist panel header with red eyebrow title + live transport metadata.
function PlaylistHeader() {
  const { bpm, sampleRate, positionSamples, timeSigNumerator, timeSigDenominator } = useTransportStore()
  const { tracks } = useTrackStore()
  const audioCount = tracks.filter(t => t.kind !== 'Master').length
  const seconds = sampleRate > 0 ? positionSamples / sampleRate : 0
  const beatsPerBar = timeSigNumerator > 0 ? timeSigNumerator : 4
  const beats = bpm > 0 ? (seconds * bpm / 60) : 0
  const bar = Math.floor(beats / beatsPerBar) + 1
  const beat = Math.floor(beats % beatsPerBar) + 1
  const tick = Math.floor((beats % 1) * 960)
  const pos = `${String(bar).padStart(3, ' ')}.${beat}.${String(tick).padStart(3, '0')}`

  return (
    <div style={{
      height: 24, flexShrink: 0,
      background: 'linear-gradient(180deg, #0a0a0d, #050507)',
      borderBottom: `1px solid ${hw.border}`,
      display: 'flex', alignItems: 'center', padding: '0 12px', gap: 12,
    }}>
      <span style={{
        fontFamily: hw.font.mono, fontSize: 10, fontWeight: 600,
        color: hw.red, letterSpacing: hw.tracking.eyebrow, textTransform: 'uppercase',
      }}>Playlist</span>
      <span style={{
        fontFamily: hw.font.mono, fontSize: 9, color: hw.textFaint,
        letterSpacing: hw.tracking.wide, textTransform: 'uppercase',
      }}>{audioCount} tracks</span>
      <span style={{ flex: 1 }} />
      <span style={hwMetaCell}>
        <span style={hwMetaLabel}>POS</span>
        <span style={hwMetaValue}>{pos}</span>
      </span>
      <span style={hwMetaCell}>
        <span style={hwMetaLabel}>BPM</span>
        <span style={hwMetaValue}>{bpm.toFixed(0)}</span>
      </span>
      <span style={hwMetaCell}>
        <span style={hwMetaLabel}>SIG</span>
        <span style={hwMetaValue}>{timeSigNumerator}/{timeSigDenominator}</span>
      </span>
    </div>
  )
}

const hwMetaCell: React.CSSProperties = {
  display: 'flex', alignItems: 'baseline', gap: 5,
  fontFamily: hw.font.mono, fontSize: 10,
  fontVariantNumeric: 'tabular-nums',
}
const hwMetaLabel: React.CSSProperties = {
  fontSize: 8, fontWeight: 600, color: hw.textFaint,
  letterSpacing: hw.tracking.eyebrow, textTransform: 'uppercase',
}
const hwMetaValue: React.CSSProperties = {
  color: hw.textPrimary, letterSpacing: hw.tracking.wide,
}

function PlaylistHintBar({ text }: { text: string }) {
  return (
    <div style={{
      height: 22, flexShrink: 0,
      background: '#040406',
      borderTop: `1px solid ${hw.border}`,
      display: 'flex', alignItems: 'center', padding: '0 12px', gap: 8,
      fontFamily: hw.font.mono, fontSize: 10, fontWeight: 500,
      color: hw.textMuted, letterSpacing: '0.02em',
      overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis',
    }}>
      <span style={{
        width: 4, height: 4, borderRadius: 2,
        background: text ? hw.red : hw.textFaint, flexShrink: 0,
      }} />
      {text || 'Playlist · drop audio to import · ctrl-wheel zoom · alt-drag bypass snap'}
    </div>
  )
}

function FloatingPanels({
  showBrowser, showPlaylist, showChannelRack, showPianoRoll, showMixer,
  onHideBrowser, onHidePlaylist, onHideChannelRack, onHidePianoRoll, onHideMixer,
}: {
  showBrowser: boolean; showPlaylist: boolean;
  showChannelRack: boolean; showPianoRoll: boolean; showMixer: boolean;
  onHideBrowser: () => void; onHidePlaylist: () => void;
  onHideChannelRack: () => void; onHidePianoRoll: () => void; onHideMixer: () => void;
}) {
  const layout = usePanelLayoutStore(s => s.layout)
  return (
    <>
      {showBrowser && layout.browser.floating && (
        <FloatingWindow panelId="browser" title="Browser" onClose={onHideBrowser}>
          <Browser />
        </FloatingWindow>
      )}
      {showPlaylist && layout.playlist.floating && (
        <FloatingWindow panelId="playlist" title="Playlist" onClose={onHidePlaylist}>
          <div style={{ flex: 1, display: 'flex', minWidth: 0, overflow: 'hidden' }}>
            <TrackList />
            <Arrangement />
          </div>
        </FloatingWindow>
      )}
      {showChannelRack && layout.channelRack.floating && (
        <FloatingWindow panelId="channelRack" title="Channel Rack" onClose={onHideChannelRack}>
          <div style={{ flex: 1, minWidth: 0 }}><ChannelRack /></div>
        </FloatingWindow>
      )}
      {showPianoRoll && layout.pianoRoll.floating && (
        <FloatingWindow panelId="pianoRoll" title="Piano Roll" onClose={onHidePianoRoll}>
          <div style={{ flex: 1, minWidth: 0 }}><PianoRoll /></div>
        </FloatingWindow>
      )}
      {showMixer && layout.mixer.floating && (
        <FloatingWindow panelId="mixer" title="Mixer" onClose={onHideMixer}>
          <div style={{ flex: 1, minWidth: 0 }}><MixerPanel /></div>
        </FloatingWindow>
      )}
    </>
  )
}
