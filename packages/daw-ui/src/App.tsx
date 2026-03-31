import { useEffect, useState } from 'react'
import { TitleBar } from './components/transport/TitleBar'
import { Toolbar } from './components/transport/Toolbar'
import { TrackList } from './components/arrangement/TrackList'
import { Arrangement } from './components/arrangement/Arrangement'
import { MixerPanel } from './components/mixer/MixerPanel'
import { Browser } from './components/browser/Browser'
import { ChannelRack } from './components/channelrack/ChannelRack'
import { Roadmap } from './components/roadmap/Roadmap'
import { useTransportStore } from './stores/transportStore'
import { useTrackStore } from './stores/trackStore'

export function App() {
  const { startListening } = useTransportStore()
  const { fetchTracks } = useTrackStore()

  // Panel visibility (FL-style toggleable panels)
  const [showBrowser, setShowBrowser] = useState(true)
  const [showMixer, setShowMixer] = useState(true)
  const [showChannelRack, setShowChannelRack] = useState(false)
  const [showPlaylist, setShowPlaylist] = useState(true)
  const [showRoadmap, setShowRoadmap] = useState(false)

  // Hint bar text (FL-style contextual help)
  const [hintText, setHintText] = useState('')

  useEffect(() => {
    startListening()
    fetchTracks()
  }, [])

  // Auto-updater
  useEffect(() => {
    async function checkUpdate() {
      try {
        const { check } = await import('@tauri-apps/plugin-updater')
        const update = await check()
        if (update) {
          const confirmed = confirm(`Update ${update.version} is available. Install now?`)
          if (confirmed) {
            await update.downloadAndInstall()
            const { relaunch } = await import('@tauri-apps/plugin-process')
            await relaunch()
          }
        }
      } catch (e) {
        console.log('Update check skipped:', e)
      }
    }
    const timer = setTimeout(checkUpdate, 3000)
    return () => clearTimeout(timer)
  }, [])

  // Global keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return

      const transport = useTransportStore.getState()
      const tracks = useTrackStore.getState()

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
        // FL-style panel toggles
        case 'F5':
          e.preventDefault()
          setShowPlaylist(v => !v)
          break
        case 'F6':
          e.preventDefault()
          setShowChannelRack(v => !v)
          break
        case 'F9':
          e.preventDefault()
          setShowMixer(v => !v)
          break
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

  return (
    <div style={{
      display: 'flex',
      flexDirection: 'column',
      height: '100vh',
      width: '100vw',
      background: '#191919',
    }}>
      {/* Title / Menu bar (FL merges these) */}
      <TitleBar hintText={hintText} />

      {/* Toolbar */}
      <Toolbar
        showBrowser={showBrowser}
        showPlaylist={showPlaylist}
        showChannelRack={showChannelRack}
        showMixer={showMixer}
        showRoadmap={showRoadmap}
        onToggleBrowser={() => setShowBrowser(v => !v)}
        onTogglePlaylist={() => setShowPlaylist(v => !v)}
        onToggleChannelRack={() => setShowChannelRack(v => !v)}
        onToggleMixer={() => setShowMixer(v => !v)}
        onToggleRoadmap={() => setShowRoadmap(v => !v)}
        onSetHint={setHintText}
      />

      {/* Main content area */}
      <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
        {showRoadmap ? (
          <Roadmap />
        ) : (
          <>
            {/* Browser panel (left) */}
            {showBrowser && <Browser />}

            {/* Center panels */}
            <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
              {/* Channel Rack */}
              {showChannelRack && (
                <div style={{ height: 200, borderBottom: '1px solid #111' }}>
                  <ChannelRack />
                </div>
              )}

              {/* Playlist / Arrangement */}
              {showPlaylist && (
                <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
                  <TrackList />
                  <Arrangement />
                </div>
              )}

              {/* Mixer */}
              {showMixer && (
                <div style={{
                  height: showPlaylist ? 220 : 'auto',
                  flex: showPlaylist ? undefined : 1,
                  borderTop: '1px solid #111',
                }}>
                  <MixerPanel />
                </div>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  )
}
