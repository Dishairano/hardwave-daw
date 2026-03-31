import { useEffect } from 'react'
import { TitleBar } from './components/transport/TitleBar'
import { Toolbar } from './components/transport/Toolbar'
import { TrackList } from './components/arrangement/TrackList'
import { Arrangement } from './components/arrangement/Arrangement'
import { MixerPanel } from './components/mixer/MixerPanel'
import { PluginBrowser } from './components/browser/PluginBrowser'
import { useTransportStore } from './stores/transportStore'
import { useTrackStore } from './stores/trackStore'

export function App() {
  const { startListening } = useTransportStore()
  const { fetchTracks } = useTrackStore()

  useEffect(() => {
    startListening()
    fetchTracks()
  }, [])

  // Global keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // Don't handle shortcuts when typing in an input
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
        case 'KeyR':
          if (!e.metaKey && !e.ctrlKey) {
            // Could toggle record in the future
          }
          break
        case 'KeyS':
          if (e.metaKey || e.ctrlKey) {
            e.preventDefault()
            // TODO: save project
          }
          break
      }
    }

    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

  return (
    <div style={{
      display: 'grid',
      gridTemplateRows: '32px 42px 1fr 200px',
      height: '100vh',
      width: '100vw',
      background: '#0a0a0b',
    }}>
      <TitleBar />
      <Toolbar />
      <div style={{
        display: 'grid',
        gridTemplateColumns: '200px 1fr 220px',
        overflow: 'hidden',
        borderTop: '1px solid rgba(255,255,255,0.06)',
      }}>
        <TrackList />
        <Arrangement />
        <PluginBrowser />
      </div>
      <MixerPanel />
    </div>
  )
}
