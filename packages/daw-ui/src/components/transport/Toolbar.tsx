import { hw } from '../../theme'
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
  const clear = () => props.onSetHint('')

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      height: 40,
      background: hw.bgElevated,
      borderBottom: `1px solid ${hw.border}`,
      padding: '0 8px',
      gap: 6,
    }}>
      {/* Transport */}
      <div style={{ display: 'flex', gap: 2 }}>
        <TransportBtn
          onClick={stop}
          active={false}
          onMouseEnter={hint('Stop')}
          onMouseLeave={clear}
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <rect x="1" y="1" width="8" height="8" rx="1" fill="currentColor" />
          </svg>
        </TransportBtn>
        <TransportBtn
          onClick={togglePlayback}
          active={playing}
          activeColor={hw.green}
          onMouseEnter={hint('Play / Pause (Space)')}
          onMouseLeave={clear}
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <polygon points="1,0 10,5 1,10" fill="currentColor" />
          </svg>
        </TransportBtn>
        <TransportBtn
          onClick={() => {}}
          active={false}
          activeColor={hw.red}
          onMouseEnter={hint('Record (R)')}
          onMouseLeave={clear}
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <circle cx="5" cy="5" r="4" fill="currentColor" />
          </svg>
        </TransportBtn>
      </div>

      <Divider />

      {/* BPM */}
      <div
        style={{
          display: 'flex', alignItems: 'center',
          background: hw.bgCard,
          border: `1px solid ${hw.border}`,
          borderRadius: hw.radius.md,
          padding: '0 8px',
          height: 26,
        }}
        onMouseEnter={hint('Tempo (BPM)')}
        onMouseLeave={clear}
      >
        <input
          type="number"
          value={bpm}
          onChange={(e) => setBpm(parseFloat(e.target.value) || 140)}
          style={{
            width: 36, background: 'transparent', border: 'none',
            color: hw.textPrimary, fontSize: 13, fontWeight: 600,
            fontFamily: "'SF Mono', 'Consolas', monospace",
            textAlign: 'right', outline: 'none',
          }}
        />
        <span style={{ fontSize: 10, color: hw.textFaint, marginLeft: 2 }}>bpm</span>
      </div>

      <Divider />

      {/* Time display */}
      <div style={{
        display: 'flex', gap: 8,
        background: hw.bgCard,
        border: `1px solid ${hw.border}`,
        borderRadius: hw.radius.md,
        padding: '4px 10px',
        height: 26,
        alignItems: 'center',
      }}>
        <span style={timeLcd}>
          {String(bar).padStart(3, ' ')}
          <span style={{ color: hw.textFaint }}>:</span>
          {beat}
          <span style={{ color: hw.textFaint }}>:</span>
          {String(tick).padStart(3, '0')}
        </span>
        <div style={{ width: 1, height: 14, background: hw.border }} />
        <span style={{ ...timeLcd, color: hw.textMuted }}>
          {String(mins).padStart(2, ' ')}
          <span style={{ color: hw.textFaint }}>:</span>
          {String(secs).padStart(2, '0')}
          <span style={{ color: hw.textFaint }}>.</span>
          {String(cs).padStart(2, '0')}
        </span>
      </div>

      <div style={{ flex: 1 }} />

      {/* Panel toggles */}
      <div style={{ display: 'flex', gap: 2 }}>
        <PanelBtn label="Browser" active={props.showBrowser} onClick={props.onToggleBrowser} onEnter={hint('Browser')} onLeave={clear} />
        <PanelBtn label="Channel" active={props.showChannelRack} onClick={props.onToggleChannelRack} onEnter={hint('Channel Rack (F6)')} onLeave={clear} />
        <PanelBtn label="Playlist" active={props.showPlaylist} onClick={props.onTogglePlaylist} onEnter={hint('Playlist (F5)')} onLeave={clear} />
        <PanelBtn label="Mixer" active={props.showMixer} onClick={props.onToggleMixer} onEnter={hint('Mixer (F9)')} onLeave={clear} />
        <PanelBtn label="Roadmap" active={props.showRoadmap} onClick={props.onToggleRoadmap} onEnter={hint('Roadmap')} onLeave={clear} />
      </div>

      <Divider />

      {/* Actions */}
      <div style={{ display: 'flex', gap: 4 }}>
        <ActionBtn label="Import" onClick={handleImport} onEnter={hint('Import audio file')} onLeave={clear} />
        <ActionBtn label="+ Track" onClick={() => addAudioTrack()} onEnter={hint('Add audio track')} onLeave={clear} />
      </div>
    </div>
  )
}

function Divider() {
  return <div style={{ width: 1, height: 18, background: hw.border, margin: '0 2px' }} />
}

function TransportBtn({ children, onClick, active, activeColor, onMouseEnter, onMouseLeave }: {
  children: React.ReactNode
  onClick: () => void
  active: boolean
  activeColor?: string
  onMouseEnter?: () => void
  onMouseLeave?: () => void
}) {
  const color = active ? (activeColor || hw.textPrimary) : hw.textMuted
  return (
    <button
      onClick={onClick}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      style={{
        width: 30, height: 26,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        color,
        background: active ? `${activeColor || hw.textPrimary}15` : hw.bgCard,
        border: `1px solid ${active ? `${activeColor || hw.textPrimary}30` : hw.border}`,
        borderRadius: hw.radius.md,
      }}
    />
  )
}

function PanelBtn({ label, active, onClick, onEnter, onLeave }: {
  label: string; active: boolean; onClick: () => void; onEnter: () => void; onLeave: () => void
}) {
  return (
    <button
      onClick={onClick}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      style={{
        padding: '3px 8px',
        fontSize: 11,
        fontWeight: 500,
        color: active ? hw.textPrimary : hw.textFaint,
        background: active ? hw.redDim : 'transparent',
        border: `1px solid ${active ? hw.red + '30' : 'transparent'}`,
        borderRadius: hw.radius.sm,
        position: 'relative',
      }}
    >
      {label}
      {active && (
        <div style={{
          position: 'absolute', bottom: -1, left: 4, right: 4,
          height: 2, background: hw.red, borderRadius: 1,
        }} />
      )}
    </button>
  )
}

function ActionBtn({ label, onClick, onEnter, onLeave }: {
  label: string; onClick: () => void; onEnter: () => void; onLeave: () => void
}) {
  return (
    <button
      onClick={onClick}
      onMouseEnter={e => {
        e.currentTarget.style.background = hw.bgHover
        e.currentTarget.style.borderColor = hw.borderStrong
        onEnter()
      }}
      onMouseLeave={e => {
        e.currentTarget.style.background = hw.bgCard
        e.currentTarget.style.borderColor = hw.border
        onLeave()
      }}
      style={{
        padding: '3px 10px', fontSize: 11, color: hw.textSecondary,
        background: hw.bgCard, border: `1px solid ${hw.border}`,
        borderRadius: hw.radius.md,
      }}
    >
      {label}
    </button>
  )
}

const timeLcd: React.CSSProperties = {
  fontFamily: "'SF Mono', 'Consolas', 'Courier New', monospace",
  fontSize: 12,
  fontWeight: 600,
  color: '#ffffff',
  letterSpacing: 0,
  whiteSpace: 'pre',
}
