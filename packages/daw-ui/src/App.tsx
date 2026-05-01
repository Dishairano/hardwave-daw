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
  const [showAbout, setShowAbout] = useState(false)

  // Splash screen
  const [showSplash, setShowSplash] = useState(true)
  const [dataReady, setDataReady] = useState(false)

  // Hint bar text — top-bar (global) + playlist (panel-local)
  const [hintText, setHintText] = useState('')
  const [playlistHint, setPlaylistHint] = useState('')

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
        onOpenAbout={() => setShowAbout(true)}
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
            <div style={{
              flex: 1, display: 'flex', flexDirection: 'column',
              overflow: 'hidden', minHeight: 80,
              background: '#000',
              borderTop: `1px solid ${hw.borderLight}`,
              position: 'relative',
            }}>
              {/* Hardwave panel signature: 2px red-gradient top stripe */}
              <div style={{
                position: 'absolute', top: 0, left: 0, right: 0, height: 2,
                background: `linear-gradient(90deg, ${hw.secondary}, ${hw.accentLight}, ${hw.secondary})`,
                zIndex: 2,
                pointerEvents: 'none',
              }} />
              {/* Panel header — JetBrains Mono uppercase title with red accent + transport metadata on the right */}
              <PlaylistHeader />
              <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 0 }}>
                <TrackList />
                <Arrangement onSetHint={setPlaylistHint} />
              </div>
              {/* Hint bar — fixed strip at the bottom of the playlist (mockup pattern) */}
              <PlaylistHintBar text={playlistHint} />
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
      {showAbout && <AboutModal onClose={() => setShowAbout(false)} />}

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

function PlaylistHeader() {
  const { bpm, sampleRate, positionSamples } = useTransportStore()
  const { tracks } = useTrackStore()
  const audioCount = tracks.filter(t => t.kind !== 'Master').length

  const seconds = sampleRate > 0 ? positionSamples / sampleRate : 0
  const beats = bpm > 0 ? (seconds * bpm / 60) : 0
  const bar = Math.floor(beats / 4) + 1
  const beat = Math.floor(beats % 4) + 1
  const tick = Math.floor((beats % 1) * 960)
  const pos = `${String(bar).padStart(3,' ')}.${beat}.${String(tick).padStart(3,'0')}`

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

      {/* Cursor position + transport meta — right-aligned, JetBrains Mono tabular */}
      <span style={metaCell}>
        <span style={metaLabel}>POS</span>
        <span style={metaValue}>{pos}</span>
      </span>
      <span style={metaCell}>
        <span style={metaLabel}>BPM</span>
        <span style={metaValue}>{bpm.toFixed(0)}</span>
      </span>
      <span style={metaCell}>
        <span style={metaLabel}>SIG</span>
        <span style={metaValue}>4/4</span>
      </span>
    </div>
  )
}

const metaCell: React.CSSProperties = {
  display: 'flex', alignItems: 'baseline', gap: 5,
  fontFamily: hw.font.mono, fontSize: 10,
  fontVariantNumeric: 'tabular-nums',
}
const metaLabel: React.CSSProperties = {
  fontSize: 8, fontWeight: 600, color: hw.textFaint,
  letterSpacing: hw.tracking.eyebrow, textTransform: 'uppercase',
}
const metaValue: React.CSSProperties = {
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
      {text || 'Playlist · drop audio to import · ctrl+wheel zoom · alt+wheel rows · alt+drag bypass snap'}
    </div>
  )
}

function AboutModal({ onClose }: { onClose: () => void }) {
  // Read version from the package — Vite makes it available via import.meta.env at build time,
  // but the canonical version lives in tauri.conf.json. Hardcoded fallback keeps it simple.
  const version = '0.157.16+'
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 200,
        background: 'rgba(0,0,0,0.6)', backdropFilter: 'blur(8px)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 420, maxWidth: '92vw',
          background: 'linear-gradient(155deg, #0d0d12 0%, #13131a 100%)',
          border: `1px solid ${hw.borderLight}`, borderRadius: 14,
          padding: '32px 28px 24px', position: 'relative',
          boxShadow: '0 24px 64px rgba(0,0,0,0.8), 0 0 1px rgba(255,255,255,0.06)',
        }}
      >
        {/* Top red stripe */}
        <div style={{
          position: 'absolute', top: 0, left: 0, right: 0, height: 3,
          background: `linear-gradient(90deg, transparent, ${hw.red}, transparent)`,
          borderRadius: '14px 14px 0 0',
        }} />

        <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: 18 }}>
          <div style={{
            width: 44, height: 44, borderRadius: 10,
            background: 'linear-gradient(145deg, #1c1c24 0%, #0a0a0e 100%)',
            border: `1px solid ${hw.borderLight}`,
            display: 'flex', alignItems: 'center', justifyContent: 'center', position: 'relative', overflow: 'hidden',
          }}>
            <div style={{
              position: 'absolute', inset: 1, borderRadius: 9,
              background: 'radial-gradient(circle at 30% 20%, rgba(239,68,68,0.25), transparent 60%)',
              pointerEvents: 'none',
            }} />
            <svg width={22} height={22} viewBox="0 0 24 24" fill="none" stroke={hw.accentLight} strokeWidth={2.4} strokeLinecap="round" strokeLinejoin="round" style={{ position: 'relative', zIndex: 1 }}>
              <path d="M12 2 L4 6 v6 c0 4.5 3.5 8.5 8 10 c4.5-1.5 8-5.5 8-10 V6 z" />
              <path d="M9 12 l2 2 l4-4" />
            </svg>
          </div>
          <div>
            <div style={{ fontFamily: hw.font.display, fontWeight: 700, fontSize: 20, color: hw.textPrimary, letterSpacing: '-0.01em' }}>
              Hardwave DAW
            </div>
            <div style={{ fontFamily: hw.font.mono, fontSize: 11, color: hw.textFaint, letterSpacing: hw.tracking.wide }}>
              v{version}
            </div>
          </div>
        </div>

        <div style={{
          fontFamily: hw.font.mono, fontSize: 11, color: hw.accentLight,
          textTransform: 'uppercase', letterSpacing: hw.tracking.eyebrow,
          marginBottom: 20,
        }}>
          Hard hours, harder hits.
        </div>

        <div style={{ fontSize: 13, color: hw.textSecondary, lineHeight: 1.6, marginBottom: 18 }}>
          The free, open-source digital audio workstation with real-time multiplayer.
          Built in Rust + Tauri. Hosts your existing VST3 and CLAP plugins.
          One Hardwave account links every plugin you own.
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 6, fontSize: 11.5, color: hw.textMuted, marginBottom: 18 }}>
          <a href="https://hardwavestudios.com" target="_blank" rel="noreferrer"
             style={{ color: hw.textMuted, textDecoration: 'none', borderBottom: `1px solid ${hw.borderLight}`, paddingBottom: 1, width: 'fit-content' }}>
            hardwavestudios.com
          </a>
          <a href="https://github.com/Dishairano/hardwave-daw" target="_blank" rel="noreferrer"
             style={{ color: hw.textMuted, textDecoration: 'none', borderBottom: `1px solid ${hw.borderLight}`, paddingBottom: 1, width: 'fit-content' }}>
            github.com/Dishairano/hardwave-daw
          </a>
          <a href="https://status.hardwavestudios.com" target="_blank" rel="noreferrer"
             style={{ color: hw.textMuted, textDecoration: 'none', borderBottom: `1px solid ${hw.borderLight}`, paddingBottom: 1, width: 'fit-content' }}>
            status.hardwavestudios.com
          </a>
        </div>

        <button
          onClick={onClose}
          style={{
            width: '100%', padding: '8px 0',
            background: 'rgba(255,255,255,0.04)',
            border: `1px solid ${hw.borderLight}`, borderRadius: 8,
            fontFamily: hw.font.ui, fontSize: 11, fontWeight: 600,
            color: hw.textSecondary, letterSpacing: hw.tracking.wide,
            textTransform: 'uppercase', cursor: 'default',
            transition: 'background 0.15s, color 0.15s',
          }}
          onMouseEnter={e => { e.currentTarget.style.background = 'rgba(255,255,255,0.08)'; e.currentTarget.style.color = hw.textPrimary }}
          onMouseLeave={e => { e.currentTarget.style.background = 'rgba(255,255,255,0.04)'; e.currentTarget.style.color = hw.textSecondary }}
        >
          Close
        </button>
      </div>
    </div>
  )
}
