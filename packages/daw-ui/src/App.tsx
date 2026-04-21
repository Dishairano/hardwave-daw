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
import { UpdateModal } from './components/UpdateModal'
import { AboutDialog } from './components/AboutDialog'
import { FloatingWindow } from './components/FloatingWindow'
import { SaveChangesDialog, type SaveChangesChoice } from './components/SaveChangesDialog'
import { TemplateDialog, type TemplateId } from './components/TemplateDialog'
import { usePanelLayoutStore } from './stores/panelLayoutStore'
import { DevPanel } from './dev/DevPanel' // DEV ONLY — remove before merge to master
import { useTransportStore } from './stores/transportStore'
import { useTrackStore } from './stores/trackStore'
import { useProjectStore } from './stores/projectStore'
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
  const [showAbout, setShowAbout] = useState(false)
  const [showDevPanel, setShowDevPanel] = useState(false) // DEV ONLY

  // Splash screen
  const [showSplash, setShowSplash] = useState(true)
  const [dataReady, setDataReady] = useState(false)

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

  useEffect(() => {
    if (initRan.current) return
    initRan.current = true
    startListening()
    fetchTracks().finally(() => setDataReady(true))
    // Check for updates after a short delay
    const timer = setTimeout(checkForUpdates, 3000)
    return () => clearTimeout(timer)
  }, [])

  // Auto-save: every 3 minutes, if project has been saved once and is dirty, save silently.
  useEffect(() => {
    const ENABLED = localStorage.getItem('hardwave.daw.autoSaveEnabled') !== 'false'
    if (!ENABLED) return
    const INTERVAL_MS = 3 * 60 * 1000
    const id = setInterval(() => {
      const { dirty, filePath } = useProjectStore.getState()
      if (dirty && filePath) {
        useProjectStore.getState().saveProject(filePath).catch(() => {})
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
    }
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
    } catch (err) {
      await showErrorDialog('Save failed', String(err))
    }
  }, [saveProject, showErrorDialog])

  const handleSaveProject = useCallback(async () => {
    try {
      await saveProject()
    } catch (err) {
      await showErrorDialog('Save failed', String(err))
    }
  }, [saveProject, showErrorDialog])

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

      // Ctrl/Cmd shortcuts
      if (e.ctrlKey || e.metaKey) {
        switch (e.key.toLowerCase()) {
          case 'n':
            e.preventDefault()
            handleNewProject()
            return
          case 'o':
            e.preventDefault()
            handleOpenProject()
            return
          case 's':
            e.preventDefault()
            if (e.shiftKey) {
              handleSaveProjectAs()
            } else {
              handleSaveProject()
            }
            return
          case 'a':
            e.preventDefault()
            tracks.selectAllClips()
            return
          case 'd':
            if (e.shiftKey) break // handled below for dev panel
            e.preventDefault()
            duplicateSelection()
            return
          case 'c':
            e.preventDefault()
            tracks.copySelectedClips()
            return
          case 'x':
            e.preventDefault()
            cutSelection()
            return
          case 'v':
            e.preventDefault()
            pasteAtPlayhead()
            return
          case 'z':
            e.preventDefault()
            if (e.shiftKey) {
              tracks.redo()
            } else {
              tracks.undo()
            }
            return
          case 'y':
            e.preventDefault()
            tracks.redo()
            return
        }
      }

      switch (e.code) {
        case 'Space':
          e.preventDefault()
          transport.togglePlayback()
          break
        case 'Delete':
        case 'Backspace':
          e.preventDefault()
          tracks.deleteSelectedClips()
          break
        case 'KeyS':
          if (e.ctrlKey || e.metaKey) break
          e.preventDefault()
          {
            const sel = tracks.selectedClipId
            if (!sel) break
            const t = tracks.tracks.find(tr => tr.clips.some(c => c.id === sel))
            if (!t) break
            const sr = transport.sampleRate || 48000
            const playheadTicks = Math.round((transport.positionSamples / sr) * (transport.bpm / 60) * 960)
            const atTicks = transport.editCursorTicks != null ? transport.editCursorTicks : playheadTicks
            const clip = t.clips.find(c => c.id === sel)
            if (clip && atTicks > clip.position_ticks && atTicks < clip.position_ticks + clip.length_ticks) {
              tracks.splitClip(t.id, sel, atTicks)
            }
          }
          break
        case 'Home':
          e.preventDefault()
          transport.setPosition(0)
          break
        case 'End':
          e.preventDefault()
          {
            const allClips = tracks.tracks.flatMap(t => t.clips || [])
            if (allClips.length > 0) {
              const lastEnd = Math.max(...allClips.map(c => c.position_ticks + c.length_ticks))
              const sr = transport.sampleRate || 48000
              const samplesPerTick = (sr * 60) / (transport.bpm * 960)
              transport.setPosition(Math.round(lastEnd * samplesPerTick))
            }
          }
          break
        case 'KeyL':
          e.preventDefault()
          transport.toggleLoop()
          break
        case 'F5':
          e.preventDefault()
          setShowPlaylist(v => !v)
          break
        case 'F6':
          e.preventDefault()
          setShowChannelRack(v => !v)
          break
        case 'F7':
          e.preventDefault()
          setShowPianoRoll(v => !v)
          break
        case 'F8':
          e.preventDefault()
          setShowBrowser(v => !v)
          break
        case 'F9':
          e.preventDefault()
          setShowMixer(v => !v)
          break
        case 'KeyD': // DEV ONLY — Ctrl+Shift+D toggles dev panel
          if (e.ctrlKey && e.shiftKey) {
            e.preventDefault()
            setShowDevPanel(v => !v)
          }
          break
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
        onCheckForUpdates={checkForUpdates}
        onToggleAbout={() => setShowAbout(v => !v)}
        onExportAudio={handleExportAudio}
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
        showChannelRack={showChannelRack}
        showPianoRoll={showPianoRoll}
        showMixer={showMixer}
        onHideBrowser={() => setShowBrowser(false)}
        onHideChannelRack={() => setShowChannelRack(false)}
        onHidePianoRoll={() => setShowPianoRoll(false)}
        onHideMixer={() => setShowMixer(false)}
      />

      {/* Floating detached panels */}
      {showRoadmap && <Roadmap onClose={() => setShowRoadmap(false)} />}
      {showAudioSettings && <AudioSettings onClose={() => setShowAudioSettings(false)} />}
      {showAbout && <AboutDialog onClose={() => setShowAbout(false)} />}
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
  return (
    <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
      {browserDocked && <div data-testid="panel-browser" style={{ width: 240, flexShrink: 0, display: 'flex' }}><Browser /></div>}

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {channelRackDocked && (
          <div data-testid="panel-channel-rack" style={{
            flex: showPlaylist ? undefined : 1,
            height: showPlaylist ? '55%' : undefined,
            minHeight: 120,
            borderBottom: showPlaylist ? `1px solid ${hw.borderDark}` : undefined,
          }}>
            <ChannelRack />
          </div>
        )}

        {pianoRollDocked && (
          <div data-testid="panel-piano-roll" style={{
            flex: 1, minHeight: 200,
            borderBottom: showPlaylist ? `1px solid ${hw.borderDark}` : undefined,
          }}>
            <PianoRoll />
          </div>
        )}

        {showPlaylist && (
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
  showBrowser, showChannelRack, showPianoRoll, showMixer,
  onHideBrowser, onHideChannelRack, onHidePianoRoll, onHideMixer,
}: {
  showBrowser: boolean; showChannelRack: boolean; showPianoRoll: boolean; showMixer: boolean;
  onHideBrowser: () => void; onHideChannelRack: () => void; onHidePianoRoll: () => void; onHideMixer: () => void;
}) {
  const layout = usePanelLayoutStore(s => s.layout)
  return (
    <>
      {showBrowser && layout.browser.floating && (
        <FloatingWindow panelId="browser" title="Browser" onClose={onHideBrowser}>
          <Browser />
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
