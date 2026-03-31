import { useTransportStore } from '../../stores/transportStore'
import { useTrackStore } from '../../stores/trackStore'

interface ToolbarProps {
  showBrowser: boolean
  showPlaylist: boolean
  showChannelRack: boolean
  showMixer: boolean
  showRoadmap: boolean
  onToggleBrowser: () => void
  onTogglePlaylist: () => void
  onToggleChannelRack: () => void
  onToggleMixer: () => void
  onToggleRoadmap: () => void
  onSetHint: (text: string) => void
}

export function Toolbar(props: ToolbarProps) {
  const { playing, bpm, positionSamples, sampleRate, togglePlayback, stop, setBpm } = useTransportStore()
  const { tracks, selectedTrackId, addAudioTrack, addMidiTrack, importAudioFile } = useTrackStore()

  const seconds = sampleRate > 0 ? positionSamples / sampleRate : 0
  const mins = Math.floor(seconds / 60)
  const secs = Math.floor(seconds % 60)
  const cs = Math.floor((seconds % 1) * 100)

  const beats = bpm > 0 ? (seconds * bpm / 60) : 0
  const bar = Math.floor(beats / 4) + 1
  const beat = Math.floor(beats % 4) + 1
  const tick = Math.floor((beats % 1) * 960)

  const handleImport = async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const selected = await open({
      multiple: false,
      filters: [{ name: 'Audio', extensions: ['wav', 'flac', 'mp3', 'ogg', 'aac', 'm4a'] }],
    })
    if (!selected) return

    let trackId = selectedTrackId
    const audioTracks = tracks.filter(t => t.kind === 'Audio')
    if (!trackId || !audioTracks.find(t => t.id === trackId)) {
      if (audioTracks.length === 0) {
        await addAudioTrack()
        const { tracks: updated } = useTrackStore.getState()
        trackId = updated.find(t => t.kind === 'Audio')?.id || null
      } else {
        trackId = audioTracks[0].id
      }
    }
    if (trackId) await importAudioFile(trackId, selected as string)
  }

  const hint = (text: string) => () => props.onSetHint(text)
  const clearHint = () => props.onSetHint('')

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      height: 32,
      background: 'linear-gradient(180deg, #3C3C3C 0%, #2C2C2C 40%, #262626 100%)',
      borderBottom: '1px solid #111',
      padding: '0 4px',
      gap: 0,
    }}>
      {/* === Shortcut icon buttons (FL-style small square buttons) === */}
      <div style={{ display: 'flex', gap: 0 }}>
        <ShortcutBtn
          icon={<BrowserIcon />}
          active={props.showBrowser}
          onClick={props.onToggleBrowser}
          onMouseEnter={hint('Toggle Browser (Alt+F8)')}
          onMouseLeave={clearHint}
        />
        <ShortcutBtn
          icon={<ChannelIcon />}
          active={props.showChannelRack}
          onClick={props.onToggleChannelRack}
          onMouseEnter={hint('Toggle Channel Rack (F6)')}
          onMouseLeave={clearHint}
        />
        <ShortcutBtn
          icon={<PlaylistIcon />}
          active={props.showPlaylist}
          onClick={props.onTogglePlaylist}
          onMouseEnter={hint('Toggle Playlist (F5)')}
          onMouseLeave={clearHint}
        />
        <ShortcutBtn
          icon={<MixerIcon />}
          active={props.showMixer}
          onClick={props.onToggleMixer}
          onMouseEnter={hint('Toggle Mixer (F9)')}
          onMouseLeave={clearHint}
        />
        <ShortcutBtn
          icon={<RoadmapIcon />}
          active={props.showRoadmap}
          onClick={props.onToggleRoadmap}
          onMouseEnter={hint('Toggle Roadmap')}
          onMouseLeave={clearHint}
        />
      </div>

      <Sep />

      {/* === Pat/Song mode toggle === */}
      <div style={{
        display: 'flex',
        background: '#1A1A1A',
        border: '1px solid #333',
        overflow: 'hidden',
      }}>
        <div
          style={{ ...modeBtn, background: '#2A2A2A', color: '#E8A030' }}
          onMouseEnter={hint('Pattern mode - play the selected pattern')}
          onMouseLeave={clearHint}
        >
          PAT
        </div>
        <div style={{ width: 1, background: '#333' }} />
        <div
          style={{ ...modeBtn, color: '#666' }}
          onMouseEnter={hint('Song mode - play the playlist arrangement')}
          onMouseLeave={clearHint}
        >
          SONG
        </div>
      </div>

      <Sep />

      {/* === Transport === */}
      <div style={{ display: 'flex', gap: 0 }}>
        {/* Stop */}
        <button
          onClick={stop}
          style={transportBtn}
          onMouseEnter={hint('Stop (Space)')}
          onMouseLeave={clearHint}
        >
          <svg width="9" height="9" viewBox="0 0 9 9">
            <rect x="0" y="0" width="9" height="9" fill="#A0A0A0" rx="1" />
          </svg>
        </button>

        {/* Play */}
        <button
          onClick={togglePlayback}
          style={{
            ...transportBtn,
            background: playing ? '#2A3A2A' : transportBtn.background,
          }}
          onMouseEnter={hint('Play / Pause (Space)')}
          onMouseLeave={clearHint}
        >
          <svg width="9" height="10" viewBox="0 0 9 10">
            <polygon points="0,0 9,5 0,10" fill={playing ? '#5A5' : '#A0A0A0'} />
          </svg>
        </button>

        {/* Record */}
        <button
          style={transportBtn}
          onMouseEnter={hint('Record (R) - Click to enable recording')}
          onMouseLeave={clearHint}
        >
          <svg width="9" height="9" viewBox="0 0 9 9">
            <circle cx="4.5" cy="4.5" r="4" fill="#8B3030" />
          </svg>
        </button>
      </div>

      <Sep />

      {/* === Pattern selector === */}
      <div style={{
        display: 'flex', alignItems: 'center',
        background: '#1A1A1A', border: '1px solid #333',
        padding: '0 2px', height: 20,
      }}>
        <button style={patArrow}>
          <span style={{ fontSize: 8, color: '#888' }}>-</span>
        </button>
        <span style={{
          fontSize: 11, fontWeight: 700, color: '#E8A030',
          fontFamily: "'Consolas', 'Courier New', monospace",
          padding: '0 4px', minWidth: 18, textAlign: 'center',
        }}>
          1
        </span>
        <button style={patArrow}>
          <span style={{ fontSize: 8, color: '#888' }}>+</span>
        </button>
      </div>

      <Sep />

      {/* === Tempo === */}
      <div
        style={{
          display: 'flex', alignItems: 'baseline',
          background: '#111', border: '1px solid #333',
          padding: '0 3px', height: 20,
        }}
        onMouseEnter={hint('Tempo - Right-click for options')}
        onMouseLeave={clearHint}
      >
        <input
          type="number"
          value={bpm}
          onChange={(e) => setBpm(parseFloat(e.target.value) || 140)}
          style={{
            width: 40, background: 'transparent', border: 'none',
            color: '#E8A030', fontSize: 13, fontWeight: 700,
            fontFamily: "'Consolas', 'Courier New', monospace",
            textAlign: 'right', outline: 'none',
          }}
        />
      </div>

      <Sep />

      {/* === Time display (FL dual: Bar:Beat:Tick | Min:Sec:Cs) === */}
      <div style={{
        display: 'flex', gap: 1,
        background: '#0D0D0D', border: '1px solid #333',
        padding: '1px 2px', height: 20,
      }}>
        {/* Bar:Beat:Tick */}
        <div style={timeDisplay}>
          <span style={{ color: '#E8A030' }}>{String(bar).padStart(3, ' ')}</span>
          <span style={{ color: '#555' }}>:</span>
          <span style={{ color: '#E8A030' }}>{beat}</span>
          <span style={{ color: '#555' }}>:</span>
          <span style={{ color: '#E8A030' }}>{String(tick).padStart(3, '0')}</span>
        </div>
        <div style={{ width: 1, background: '#333' }} />
        {/* Min:Sec:Centisecond */}
        <div style={timeDisplay}>
          <span style={{ color: '#5A8A5A' }}>{String(mins).padStart(2, ' ')}</span>
          <span style={{ color: '#444' }}>:</span>
          <span style={{ color: '#5A8A5A' }}>{String(secs).padStart(2, '0')}</span>
          <span style={{ color: '#444' }}>:</span>
          <span style={{ color: '#5A8A5A' }}>{String(cs).padStart(2, '0')}</span>
        </div>
      </div>

      <Sep />

      {/* === Snap selector === */}
      <div style={{
        background: '#1A1A1A', border: '1px solid #333',
        padding: '0 6px', height: 20, display: 'flex', alignItems: 'center',
      }}>
        <span style={{ fontSize: 9, color: '#888' }}>Line</span>
      </div>

      <div style={{ flex: 1 }} />

      {/* === Right side: action buttons === */}
      <div style={{ display: 'flex', gap: 1 }}>
        <ActionBtn label="Import" onClick={handleImport} onHint={hint('Import audio file')} onClear={clearHint} />
        <ActionBtn label="+Audio" onClick={() => addAudioTrack()} onHint={hint('Add audio track')} onClear={clearHint} />
        <ActionBtn label="+MIDI" onClick={() => addMidiTrack()} onHint={hint('Add MIDI instrument track')} onClear={clearHint} />
      </div>

      <Sep />

      {/* === Output mini meter (FL has oscilloscope here) === */}
      <div style={{
        width: 60, height: 20,
        background: '#0D0D0D', border: '1px solid #333',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}>
        <span style={{ fontSize: 8, color: '#333' }}>scope</span>
      </div>

      <Sep />

      {/* === CPU display === */}
      <div style={{
        background: '#0D0D0D', border: '1px solid #333',
        padding: '0 6px', height: 20,
        display: 'flex', alignItems: 'center', gap: 4,
      }}>
        <span style={{ fontSize: 9, color: '#5A8A5A' }}>0%</span>
        <span style={{ fontSize: 8, color: '#444' }}>cpu</span>
      </div>
    </div>
  )
}

function Sep() {
  return <div style={{ width: 1, height: 20, background: '#444', margin: '0 3px', opacity: 0.5 }} />
}

/* FL-style small square shortcut buttons */
function ShortcutBtn({ icon, active, onClick, onMouseEnter, onMouseLeave }: {
  icon: React.ReactNode
  active: boolean
  onClick: () => void
  onMouseEnter?: () => void
  onMouseLeave?: () => void
}) {
  return (
    <button
      onClick={onClick}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      style={{
        width: 22, height: 22,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: active ? 'rgba(232,160,48,0.15)' : 'transparent',
        border: '1px solid transparent',
        borderColor: active ? '#E8A030' : 'transparent',
        borderRadius: 0,
      }}
    >
      {icon}
    </button>
  )
}

/* Tiny SVG icons for shortcut buttons */
function BrowserIcon() {
  return <svg width="12" height="12" viewBox="0 0 12 12"><rect x="1" y="1" width="10" height="10" rx="1" fill="none" stroke="#999" strokeWidth="1"/><line x1="4" y1="1" x2="4" y2="11" stroke="#999" strokeWidth="1"/></svg>
}
function ChannelIcon() {
  return <svg width="12" height="12" viewBox="0 0 12 12"><rect x="1" y="2" width="10" height="2" rx="0.5" fill="#999"/><rect x="1" y="5" width="10" height="2" rx="0.5" fill="#999"/><rect x="1" y="8" width="10" height="2" rx="0.5" fill="#999"/></svg>
}
function PlaylistIcon() {
  return <svg width="12" height="12" viewBox="0 0 12 12"><rect x="1" y="2" width="4" height="3" rx="0.5" fill="#999"/><rect x="6" y="2" width="5" height="3" rx="0.5" fill="#777"/><rect x="2" y="7" width="6" height="3" rx="0.5" fill="#999"/></svg>
}
function MixerIcon() {
  return <svg width="12" height="12" viewBox="0 0 12 12"><line x1="3" y1="2" x2="3" y2="10" stroke="#999" strokeWidth="1.5"/><line x1="6" y1="2" x2="6" y2="10" stroke="#999" strokeWidth="1.5"/><line x1="9" y1="2" x2="9" y2="10" stroke="#999" strokeWidth="1.5"/><circle cx="3" cy="4" r="1.5" fill="#BBB"/><circle cx="6" cy="7" r="1.5" fill="#BBB"/><circle cx="9" cy="5" r="1.5" fill="#BBB"/></svg>
}
function RoadmapIcon() {
  return <svg width="12" height="12" viewBox="0 0 12 12"><path d="M2 3h8M2 6h6M2 9h4" stroke="#999" strokeWidth="1.5" strokeLinecap="round"/></svg>
}

function ActionBtn({ label, onClick, onHint, onClear }: {
  label: string; onClick: () => void; onHint: () => void; onClear: () => void
}) {
  return (
    <button
      onClick={onClick}
      onMouseEnter={onHint}
      onMouseLeave={onClear}
      style={{
        padding: '2px 8px', fontSize: 10, color: '#999',
        background: '#252525', border: '1px solid #3A3A3A',
      }}
    >
      {label}
    </button>
  )
}

const transportBtn: React.CSSProperties = {
  width: 24, height: 22,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: '#1E1E1E',
  border: '1px solid #3A3A3A',
}

const modeBtn: React.CSSProperties = {
  padding: '2px 6px',
  fontSize: 9,
  fontWeight: 700,
  cursor: 'default',
  letterSpacing: 0.5,
}

const timeDisplay: React.CSSProperties = {
  fontFamily: "'Consolas', 'Courier New', monospace",
  fontSize: 12,
  fontWeight: 700,
  padding: '0 3px',
  letterSpacing: 0,
  whiteSpace: 'pre',
}

const patArrow: React.CSSProperties = {
  width: 14, height: 18,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: 'transparent', border: 'none',
}
