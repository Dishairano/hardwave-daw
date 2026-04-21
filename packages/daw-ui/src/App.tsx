import { useEffect, useState, useRef, useCallback } from 'react'
import { SplashScreen } from './components/SplashScreen'
import { TitleBar } from './components/transport/TitleBar'
import { Toolbar } from './components/transport/Toolbar'
import { TrackList } from './components/arrangement/TrackList'
import { Arrangement } from './components/arrangement/Arrangement'
import { MixerPanel } from './components/mixer/MixerPanel'
import { Browser } from './components/browser/Browser'
import { ChannelRack } from './components/channelrack/ChannelRack'
import { PianoRoll } from './components/piano-roll/PianoRoll'
import { Roadmap } from './components/roadmap/Roadmap'
import { AudioSettings } from './components/settings/AudioSettings'
import { ThemePicker } from './components/settings/ThemePicker'
import { UpdateModal } from './components/UpdateModal'
import { AboutDialog } from './components/AboutDialog'
import { FloatingWindow } from './components/FloatingWindow'
import { SaveChangesDialog, type SaveChangesChoice } from './components/SaveChangesDialog'
import { TemplateDialog, type TemplateId } from './components/TemplateDialog'
import { useUserTemplateStore } from './stores/userTemplateStore'
import { WelcomeScreen, shouldSkipWelcome } from './components/WelcomeScreen'
import { NotificationHost } from './components/NotificationHost'
import { MetronomeScheduler } from './components/transport/MetronomeScheduler'
import { CrashRecoveryDialog, type CrashChoice } from './components/CrashRecoveryDialog'
import { ShortcutsPanel } from './components/ShortcutsPanel'
import { SampleEditor } from './components/sample-editor/SampleEditor'
import { useSampleEditorStore } from './stores/sampleEditorStore'
import { BeatSlicer } from './components/beat-slicer/BeatSlicer'
import { useBeatSlicerStore } from './stores/beatSlicerStore'
import { LoudnessMeter } from './components/LoudnessMeter'
import { HistoryPanel } from './components/HistoryPanel'
import { PrecountOverlay } from './components/transport/PrecountOverlay'
import { invoke } from '@tauri-apps/api/core'
import { usePanelLayoutStore } from './stores/panelLayoutStore'
import { DevPanel } from './dev/DevPanel' // DEV ONLY — remove before merge to master
import { useTransportStore } from './stores/transportStore'
import { useTrackStore } from './stores/trackStore'
import { useProjectStore } from './stores/projectStore'
import { useShortcutsStore } from './stores/shortcutsStore'
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
}

export function App() {
  const { startListening } = useTransportStore()
  const { fetchTracks } = useTrackStore()
  const { newProject, saveProject, loadProject } = useProjectStore()
  const recentProjects = useProjectStore(s => s.recentProjects)

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
  const [showLoudness, setShowLoudness] = useState(false)
  const [showHistory, setShowHistory] = useState(false)
  const sampleEditorPath = useSampleEditorStore(s => s.openPath)
  const closeSampleEditor = useSampleEditorStore(s => s.close)
  const beatSlicerPath = useBeatSlicerStore(s => s.openPath)
  const closeBeatSlicer = useBeatSlicerStore(s => s.close)
  const [showDevPanel, setShowDevPanel] = useState(false) // DEV ONLY

  // Splash screen
  const [showSplash, setShowSplash] = useState(true)
  const [dataReady, setDataReady] = useState(false)

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

  // Open Piano Roll on request from arrangement double-click.
  useEffect(() => {
    const onOpen = () => setShowPianoRoll(true)
    window.addEventListener('daw:openPianoRoll', onOpen)
    return () => window.removeEventListener('daw:openPianoRoll', onOpen)
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
  })
  const initRan = useRef(false)

  const customBg = useThemeStore(s => s.customBg)
  useEffect(() => { applyCustomBg(customBg) }, [customBg])

  useEffect(() => {
    if (initRan.current) return
    initRan.current = true
    startListening()
    fetchTracks().finally(() => setDataReady(true))

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

    // Check for updates after a short delay
    const timer = setTimeout(checkForUpdates, 3000)
    return () => clearTimeout(timer)
  }, [])

  // Auto-save: every 2 minutes, if project is dirty, write to the cache dir.
  // Keeps the last 3 snapshots. Independent of any user-chosen file path.
  useEffect(() => {
    const ENABLED = localStorage.getItem('hardwave.daw.autoSaveEnabled') !== 'false'
    if (!ENABLED) return
    const INTERVAL_MS = 2 * 60 * 1000
    const id = setInterval(() => {
      const { dirty } = useProjectStore.getState()
      if (dirty) {
        invoke('autosave_save').catch(() => {})
      }
    }, INTERVAL_MS)
    return () => clearInterval(id)
  }, [])

  // Warn before closing the window if there are unsaved changes.
  useEffect(() => {
    const handler = (e: BeforeUnloadEvent) => {
      if (useProjectStore.getState().dirty) {
        e.preventDefault()
        e.returnValue = ''
      }
    }
    window.addEventListener('beforeunload', handler)
    return () => window.removeEventListener('beforeunload', handler)
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
      for (let i = 1; i <= 8; i++) await ts.addAudioTrack(`Track ${i}`)
      await ts.addAudioTrack('Bus A')
      await ts.addAudioTrack('Bus B')
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
      } catch (err) {
        useProjectStore.getState().removeRecent(path)
        await showErrorDialog('Could not open project', `${path}\n\n${err}`)
      }
    } catch {}
  }, [loadProject, fetchTracks, confirmDiscardIfDirty, showErrorDialog])

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
        }
      } catch {}
    }
  }, [crashInfo, loadProject, fetchTracks, showErrorDialog])

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
    } catch (err) {
      useProjectStore.getState().removeRecent(path)
      await showErrorDialog('Could not open project', `${path}\n\nRemoved from recent projects.\n\n${err}`)
    }
  }, [loadProject, fetchTracks, confirmDiscardIfDirty, showErrorDialog])

  const handleExportAudio = useCallback(async () => {
    try {
      const { message } = await import('@tauri-apps/plugin-dialog')
      await message('Audio export is coming in a future release.', {
        title: 'Export audio',
        kind: 'info',
      })
    } catch {}
  }, [])

  const handleAddAutomationTrack = useCallback(async () => {
    try {
      const { message } = await import('@tauri-apps/plugin-dialog')
      await message('Automation tracks are coming in a future release.', {
        title: 'Add automation track',
        kind: 'info',
      })
    } catch {}
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

      // F1 is a dedicated help key — always opens the shortcuts panel as the canonical entry point.
      if (e.code === 'F1') {
        e.preventDefault()
        setShowShortcuts(v => !v)
        return
      }

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
        case 'togglePlaylist':        setShowPlaylist(v => !v); return
        case 'toggleChannelRack':     setShowChannelRack(v => !v); return
        case 'togglePianoRoll':       setShowPianoRoll(v => !v); return
        case 'toggleBrowser':         setShowBrowser(v => !v); return
        case 'toggleMixer':           setShowMixer(v => !v); return
        case 'toggleShortcutsPanel':  setShowShortcuts(v => !v); return
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [handleNewProject, handleOpenProject, handleSaveProject, handleSaveProjectAs, fetchTracks, duplicateSelection, cutSelection, pasteAtPlayhead])

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
          onFinished={() => setShowSplash(false)}
        />
      )}

      <TitleBar
        hintText={hintText}
        onNewProject={handleNewProject}
        onSaveProject={handleSaveProject}
        onSaveProjectAs={handleSaveProjectAs}
        onOpenProject={handleOpenProject}
        onUndo={() => useTrackStore.getState().undo()}
        onRedo={() => useTrackStore.getState().redo()}
        onCut={cutSelection}
        onCopy={() => useTrackStore.getState().copySelectedClips()}
        onPaste={pasteAtPlayhead}
        onDuplicate={duplicateSelection}
        onSelectAll={() => useTrackStore.getState().selectAllClips()}
        onAddAudioTrack={() => useTrackStore.getState().addAudioTrack()}
        onAddInstrumentTrack={() => useTrackStore.getState().addMidiTrack()}
        onAddAutomationTrack={handleAddAutomationTrack}
        onToggleBrowser={() => setShowBrowser(v => !v)}
        onTogglePlaylist={() => setShowPlaylist(v => !v)}
        onToggleChannelRack={() => setShowChannelRack(v => !v)}
        onTogglePianoRoll={() => setShowPianoRoll(v => !v)}
        onToggleMixer={() => setShowMixer(v => !v)}
        onToggleRoadmap={() => setShowRoadmap(v => !v)}
        onOpenAudioSettings={() => setShowAudioSettings(true)}
        onOpenThemePicker={() => setShowThemePicker(true)}
        onOpenLoudness={() => setShowLoudness(true)}
        onCheckForUpdates={checkForUpdates}
        onToggleAbout={() => setShowAbout(v => !v)}
        onToggleShortcuts={() => setShowShortcuts(v => !v)}
        onOpenHistory={() => setShowHistory(true)}
        onExportAudio={handleExportAudio}
        onSaveAsTemplate={handleSaveAsTemplate}
        onAutoCrossfade={async () => {
          const pairs = await useTrackStore.getState().autoCrossfadeOverlaps()
          const notify = useNotificationStore.getState().push
          if (pairs === 0) notify('info', 'No overlapping clips found')
          else notify('info', `Auto-crossfaded ${pairs} overlap${pairs === 1 ? '' : 's'}`)
        }}
        recentProjects={recentProjects}
        onOpenRecentProject={handleOpenRecent}
        onClearRecentProjects={() => useProjectStore.getState().clearRecent()}
        showBrowser={showBrowser}
        showPlaylist={showPlaylist}
        showChannelRack={showChannelRack}
        showPianoRoll={showPianoRoll}
        showMixer={showMixer}
      />

      <Toolbar
        showBrowser={showBrowser}
        showPlaylist={showPlaylist}
        showChannelRack={showChannelRack}
        showPianoRoll={showPianoRoll}
        showMixer={showMixer}
        onToggleBrowser={() => setShowBrowser(v => !v)}
        onTogglePlaylist={() => setShowPlaylist(v => !v)}
        onToggleChannelRack={() => setShowChannelRack(v => !v)}
        onTogglePianoRoll={() => setShowPianoRoll(v => !v)}
        onToggleMixer={() => setShowMixer(v => !v)}
        onSetHint={setHintText}
      />

      <MainLayout
        showBrowser={showBrowser}
        showPlaylist={showPlaylist}
        showChannelRack={showChannelRack}
        showPianoRoll={showPianoRoll}
        showMixer={showMixer}
      />

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
      {showAbout && <AboutDialog onClose={() => setShowAbout(false)} />}
      <ShortcutsPanel open={showShortcuts} onClose={() => setShowShortcuts(false)} />
      {sampleEditorPath && <SampleEditor path={sampleEditorPath} onClose={closeSampleEditor} />}
      {beatSlicerPath && <BeatSlicer path={beatSlicerPath} onClose={closeBeatSlicer} />}
      {showLoudness && <LoudnessMeter onClose={() => setShowLoudness(false)} />}

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
          onUpdate={handleUpdate}
          onDismiss={handleDismissUpdate}
        />
      )}
    </div>
  )
}

function MainLayout({
  showBrowser, showPlaylist, showChannelRack, showPianoRoll, showMixer,
}: {
  showBrowser: boolean; showPlaylist: boolean;
  showChannelRack: boolean; showPianoRoll: boolean; showMixer: boolean;
}) {
  const layout = usePanelLayoutStore(s => s.layout)
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
          <div data-testid="panel-playlist" style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 80 }}>
            <TrackList />
            <Arrangement />
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
