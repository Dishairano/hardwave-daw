import { useTransportStore } from '../../stores/transportStore'
import { useTrackStore } from '../../stores/trackStore'

interface ToolbarProps {
  showBrowser: boolean
  showPlaylist: boolean
  showChannelRack: boolean
  showMixer: boolean
  onToggleBrowser: () => void
  onTogglePlaylist: () => void
  onToggleChannelRack: () => void
  onToggleMixer: () => void
}

export function Toolbar(props: ToolbarProps) {
  const { playing, bpm, positionSamples, sampleRate, togglePlayback, stop, setBpm } = useTransportStore()
  const { tracks, selectedTrackId, addAudioTrack, addMidiTrack, importAudioFile } = useTrackStore()

  const seconds = sampleRate > 0 ? positionSamples / sampleRate : 0
  const mins = Math.floor(seconds / 60)
  const secs = Math.floor(seconds % 60)
  const ms = Math.floor((seconds % 1) * 1000)

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

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      height: 38,
      background: 'linear-gradient(180deg, #3A3A3A, #2E2E2E)',
      borderBottom: '1px solid #1A1A1A',
      padding: '0 6px',
      gap: 2,
    }}>
      {/* Panel toggle buttons (FL-style) */}
      <ToggleBtn label="Browser" active={props.showBrowser} onClick={props.onToggleBrowser} />
      <ToggleBtn label="Channel" active={props.showChannelRack} onClick={props.onToggleChannelRack} />
      <ToggleBtn label="Playlist" active={props.showPlaylist} onClick={props.onTogglePlaylist} />
      <ToggleBtn label="Mixer" active={props.showMixer} onClick={props.onToggleMixer} />

      <Separator />

      {/* Transport controls (FL-style) */}
      <div style={{ display: 'flex', gap: 1 }}>
        {/* Stop */}
        <button onClick={stop} style={transportBtn} title="Stop">
          <svg width="10" height="10" viewBox="0 0 10 10">
            <rect x="1" y="1" width="8" height="8" fill="#C8C8C8" />
          </svg>
        </button>

        {/* Play */}
        <button
          onClick={togglePlayback}
          style={{ ...transportBtn, background: playing ? '#3A5A3A' : transportBtn.background }}
          title="Play / Pause"
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <polygon points="1,0 10,5 1,10" fill={playing ? '#4CAF50' : '#C8C8C8'} />
          </svg>
        </button>

        {/* Record */}
        <button style={transportBtn} title="Record">
          <svg width="10" height="10" viewBox="0 0 10 10">
            <circle cx="5" cy="5" r="4" fill="#AA3333" />
          </svg>
        </button>
      </div>

      <Separator />

      {/* Pattern / Song mode selector */}
      <div style={{
        display: 'flex',
        background: '#222',
        borderRadius: 3,
        border: '1px solid #444',
        overflow: 'hidden',
      }}>
        <div style={{ ...modeBtn, background: '#333', color: '#FF6B00' }}>PAT</div>
        <div style={{ ...modeBtn, color: '#888' }}>SONG</div>
      </div>

      <Separator />

      {/* Time display (FL-style dual display) */}
      <div style={{
        display: 'flex',
        gap: 1,
        background: '#111',
        borderRadius: 3,
        border: '1px solid #444',
        padding: 2,
      }}>
        {/* Bar:Beat:Tick */}
        <div style={timeDisplay}>
          <span style={{ color: '#FF6B00' }}>{String(bar).padStart(3, ' ')}</span>
          <span style={{ color: '#666' }}>:</span>
          <span style={{ color: '#FF6B00' }}>{beat}</span>
          <span style={{ color: '#666' }}>:</span>
          <span style={{ color: '#FF6B00' }}>{String(tick).padStart(3, '0')}</span>
        </div>
        {/* Min:Sec:Ms */}
        <div style={timeDisplay}>
          <span style={{ color: '#6A9F6A' }}>{String(mins).padStart(2, ' ')}</span>
          <span style={{ color: '#555' }}>:</span>
          <span style={{ color: '#6A9F6A' }}>{String(secs).padStart(2, '0')}</span>
          <span style={{ color: '#555' }}>:</span>
          <span style={{ color: '#6A9F6A' }}>{String(ms).padStart(3, '0')}</span>
        </div>
      </div>

      <Separator />

      {/* Tempo */}
      <div style={{
        display: 'flex', alignItems: 'center',
        background: '#111', borderRadius: 3, border: '1px solid #444',
        padding: '2px 6px',
      }}>
        <input
          type="number"
          value={bpm}
          onChange={(e) => setBpm(parseFloat(e.target.value) || 140)}
          style={{
            width: 48, background: 'transparent', border: 'none',
            color: '#FF6B00', fontSize: 14, fontWeight: 700,
            fontFamily: "'Courier New', monospace", textAlign: 'right',
            outline: 'none',
          }}
        />
        <span style={{ fontSize: 8, color: '#666', marginLeft: 2 }}>BPM</span>
      </div>

      <div style={{ flex: 1 }} />

      {/* Right side controls */}
      <button onClick={handleImport} style={actionBtn} title="Import audio file">
        Import
      </button>
      <button onClick={() => addAudioTrack()} style={actionBtn} title="Add audio track">
        + Audio
      </button>
      <button onClick={() => addMidiTrack()} style={actionBtn} title="Add MIDI track">
        + MIDI
      </button>

      {/* CPU / hint display (FL-style) */}
      <div style={{
        marginLeft: 8, padding: '2px 8px',
        background: '#111', borderRadius: 3, border: '1px solid #333',
        fontSize: 9, color: '#666', minWidth: 120, textAlign: 'center',
      }}>
        Hardwave DAW
      </div>
    </div>
  )
}

function Separator() {
  return <div style={{ width: 1, height: 22, background: '#444', margin: '0 4px' }} />
}

function ToggleBtn({ label, active, onClick }: { label: string, active: boolean, onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      style={{
        padding: '3px 8px',
        fontSize: 9,
        fontWeight: 600,
        color: active ? '#FF6B00' : '#777',
        background: active ? 'rgba(255,107,0,0.12)' : 'transparent',
        border: `1px solid ${active ? '#FF6B00' : '#444'}`,
        borderRadius: 3,
      }}
    >
      {label}
    </button>
  )
}

const transportBtn: React.CSSProperties = {
  width: 26, height: 26,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: '#252525',
  border: '1px solid #444',
  borderRadius: 3,
}

const modeBtn: React.CSSProperties = {
  padding: '3px 8px',
  fontSize: 9,
  fontWeight: 700,
  cursor: 'pointer',
  letterSpacing: 0.5,
}

const timeDisplay: React.CSSProperties = {
  fontFamily: "'Courier New', monospace",
  fontSize: 13,
  fontWeight: 700,
  padding: '1px 6px',
  letterSpacing: 0,
  whiteSpace: 'pre',
}

const actionBtn: React.CSSProperties = {
  padding: '3px 10px',
  fontSize: 10,
  color: '#AAA',
  background: '#2A2A2A',
  border: '1px solid #444',
  borderRadius: 3,
}
