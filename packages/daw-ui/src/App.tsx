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

  // Panel visibility
  const [showBrowser, setShowBrowser] = useState(true)
  const [showMixer, setShowMixer] = useState(false)
  const [showChannelRack, setShowChannelRack] = useState(false)
  const [showPlaylist, setShowPlaylist] = useState(true)
  const [showPianoRoll, setShowPianoRoll] = useState(false)
  const [showRoadmap, setShowRoadmap] = useState(false)
  const [showAudioSettings, setShowAudioSettings] = useState(false)

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

  const handleOpenProject = useCallback(async () => {
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
  }, [loadProject, fetchTracks])

  const handleSaveProjectAs = useCallback(async () => {
    try {
      await saveProject(undefined)
    } catch {}
  }, [saveProject])

  // Global keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return

      const transport = useTransportStore.getState()
      const tracks = useTrackStore.getState()
      const project = useProjectStore.getState()

      // Ctrl/Cmd shortcuts
      if (e.ctrlKey || e.metaKey) {
        switch (e.key.toLowerCase()) {
          case 'n':
            e.preventDefault()
            project.newProject().then(() => fetchTracks())
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
              project.saveProject()
            }
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
          tracks.deleteSelectedClip()
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
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [handleOpenProject, handleSaveProjectAs, fetchTracks])

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
        onNewProject={() => newProject().then(() => fetchTracks())}
        onSaveProject={() => saveProject()}
        onSaveProjectAs={handleSaveProjectAs}
        onOpenProject={handleOpenProject}
        onUndo={() => {}}
        onRedo={() => {}}
        onToggleBrowser={() => setShowBrowser(v => !v)}
        onTogglePlaylist={() => setShowPlaylist(v => !v)}
        onToggleChannelRack={() => setShowChannelRack(v => !v)}
        onTogglePianoRoll={() => setShowPianoRoll(v => !v)}
        onToggleMixer={() => setShowMixer(v => !v)}
        onToggleRoadmap={() => setShowRoadmap(v => !v)}
        onOpenAudioSettings={() => setShowAudioSettings(true)}
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

      <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
        {showBrowser && <Browser />}

        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
          {showChannelRack && (
            <div style={{
              flex: showPlaylist ? undefined : 1,
              height: showPlaylist ? '55%' : undefined,
              minHeight: 120,
              borderBottom: showPlaylist ? `1px solid ${hw.borderDark}` : undefined,
            }}>
              <ChannelRack />
            </div>
          )}

          {showPianoRoll && (
            <div style={{
              flex: 1, minHeight: 200,
              borderBottom: showPlaylist ? `1px solid ${hw.borderDark}` : undefined,
            }}>
              <PianoRoll />
            </div>
          )}

          {showPlaylist && (
            <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 80 }}>
              <TrackList />
              <Arrangement />
            </div>
          )}

          {showMixer && (
            <div style={{
              height: (showPlaylist || showChannelRack || showPianoRoll) ? 220 : 'auto',
              flex: (showPlaylist || showChannelRack || showPianoRoll) ? undefined : 1,
              borderTop: `1px solid ${hw.borderDark}`,
            }}>
              <MixerPanel />
            </div>
          )}
        </div>
      </div>

      {/* Floating detached panels */}
      {showRoadmap && <Roadmap onClose={() => setShowRoadmap(false)} />}
      {showAudioSettings && <AudioSettings onClose={() => setShowAudioSettings(false)} />}

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
