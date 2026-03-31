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
      height: 34,
      background: hw.bgToolbarGrad,
      borderBottom: `1px solid ${hw.borderDark}`,
      padding: '0 4px',
      gap: 1,
    }}>
      {/* Panel shortcut buttons (small icon buttons) */}
      <div style={{ display: 'flex', gap: 0 }}>
        <IconBtn icon="browser" active={props.showBrowser} onClick={props.onToggleBrowser} onEnter={hint('Browser')} onLeave={clear} />
        <IconBtn icon="channel" active={props.showChannelRack} onClick={props.onToggleChannelRack} onEnter={hint('Channel Rack (F6)')} onLeave={clear} />
        <IconBtn icon="playlist" active={props.showPlaylist} onClick={props.onTogglePlaylist} onEnter={hint('Playlist (F5)')} onLeave={clear} />
        <IconBtn icon="mixer" active={props.showMixer} onClick={props.onToggleMixer} onEnter={hint('Mixer (F9)')} onLeave={clear} />
        <IconBtn icon="roadmap" active={props.showRoadmap} onClick={props.onToggleRoadmap} onEnter={hint('Roadmap')} onLeave={clear} />
      </div>

      <Sep />

      {/* PAT / SONG */}
      <div style={{
        display: 'flex', background: hw.bgInput, border: `1px solid ${hw.border}`,
        overflow: 'hidden', borderRadius: 2,
      }}>
        <ModeBtn label="PAT" active onEnter={hint('Pattern mode')} onLeave={clear} />
        <div style={{ width: 1, background: hw.border }} />
        <ModeBtn label="SONG" active={false} onEnter={hint('Song mode')} onLeave={clear} />
      </div>

      <Sep />

      {/* Transport */}
      <div style={{ display: 'flex', gap: 0 }}>
        <button onClick={stop} style={tBtn} onMouseEnter={hint('Stop')} onMouseLeave={clear}>
          <svg width="8" height="8"><rect width="8" height="8" rx="1" fill={hw.textMuted} /></svg>
        </button>
        <button onClick={togglePlayback} style={{ ...tBtn, background: playing ? hw.greenDim : tBtn.background }} onMouseEnter={hint('Play (Space)')} onMouseLeave={clear}>
          <svg width="8" height="10"><polygon points="0,0 8,5 0,10" fill={playing ? hw.green : hw.textMuted} /></svg>
        </button>
        <button style={tBtn} onMouseEnter={hint('Record (R)')} onMouseLeave={clear}>
          <svg width="8" height="8"><circle cx="4" cy="4" r="3.5" fill="#8B3030" /></svg>
        </button>
      </div>

      <Sep />

      {/* Tempo */}
      <div style={{ ...lcdBox, width: 52 }} onMouseEnter={hint('Tempo')} onMouseLeave={clear}>
        <input
          type="number" value={bpm}
          onChange={e => setBpm(parseFloat(e.target.value) || 140)}
          style={{
            width: 34, background: 'transparent', border: 'none',
            color: hw.purple, fontSize: 13, fontWeight: 700,
            fontFamily: "'Consolas', 'Courier New', monospace",
            textAlign: 'right', outline: 'none',
          }}
        />
      </div>

      <Sep />

      {/* Large centered time display — like FL VEEL shows prominently */}
      <div style={{
        ...lcdBox, padding: '0 6px', gap: 6,
      }}>
        {/* Bar:Beat:Tick */}
        <span style={lcdText}>
          <span style={{ color: hw.purple }}>{String(bar).padStart(3, ' ')}</span>
          <span style={{ color: hw.textFaint }}>:</span>
          <span style={{ color: hw.purple }}>{beat}</span>
          <span style={{ color: hw.textFaint }}>:</span>
          <span style={{ color: hw.purple }}>{String(tick).padStart(3, '0')}</span>
        </span>
        <div style={{ width: 1, height: 12, background: hw.border }} />
        {/* Min:Sec.Cs */}
        <span style={lcdText}>
          <span style={{ color: hw.green }}>{String(mins).padStart(2, ' ')}</span>
          <span style={{ color: hw.textFaint }}>:</span>
          <span style={{ color: hw.green }}>{String(secs).padStart(2, '0')}</span>
          <span style={{ color: hw.textFaint }}>.</span>
          <span style={{ color: hw.green }}>{String(cs).padStart(2, '0')}</span>
        </span>
      </div>

      <Sep />

      {/* Snap */}
      <div style={{ ...lcdBox, padding: '0 6px' }}>
        <span style={{ fontSize: 10, color: hw.textMuted }}>Line</span>
      </div>

      <div style={{ flex: 1 }} />

      {/* Right side actions */}
      <div style={{ display: 'flex', gap: 2 }}>
        <SmallBtn label="Import" onClick={handleImport} onEnter={hint('Import audio file')} onLeave={clear} />
        <SmallBtn label="+Audio" onClick={() => addAudioTrack()} onEnter={hint('Add audio track')} onLeave={clear} />
        <SmallBtn label="+MIDI" onClick={() => addMidiTrack()} onEnter={hint('Add MIDI track')} onLeave={clear} />
      </div>

      <Sep />

      {/* Mini scope placeholder */}
      <div style={{
        width: 56, height: 20,
        background: hw.bgDeep, border: `1px solid ${hw.border}`,
        borderRadius: 2,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}>
        <span style={{ fontSize: 7, color: hw.textFaint }}>SCOPE</span>
      </div>
    </div>
  )
}

/* --- Sub-components --- */

function Sep() {
  return <div style={{ width: 1, height: 20, background: hw.border, margin: '0 3px' }} />
}

function IconBtn({ icon, active, onClick, onEnter, onLeave }: {
  icon: string; active: boolean; onClick: () => void; onEnter: () => void; onLeave: () => void
}) {
  const icons: Record<string, React.ReactNode> = {
    browser: <svg width="11" height="11" viewBox="0 0 11 11"><rect x="0.5" y="0.5" width="10" height="10" rx="1" fill="none" stroke="currentColor" strokeWidth="1"/><line x1="3.5" y1="0.5" x2="3.5" y2="10.5" stroke="currentColor" strokeWidth="1"/></svg>,
    channel: <svg width="11" height="11" viewBox="0 0 11 11"><rect x="0.5" y="1.5" width="10" height="2" rx="0.5" fill="currentColor"/><rect x="0.5" y="4.5" width="10" height="2" rx="0.5" fill="currentColor"/><rect x="0.5" y="7.5" width="10" height="2" rx="0.5" fill="currentColor"/></svg>,
    playlist: <svg width="11" height="11" viewBox="0 0 11 11"><rect x="0.5" y="1" width="4" height="3" rx="0.5" fill="currentColor"/><rect x="5.5" y="1" width="5" height="3" rx="0.5" fill="currentColor" opacity="0.6"/><rect x="1.5" y="6" width="6" height="3" rx="0.5" fill="currentColor"/></svg>,
    mixer: <svg width="11" height="11" viewBox="0 0 11 11"><line x1="2.5" y1="1" x2="2.5" y2="10" stroke="currentColor" strokeWidth="1.2"/><line x1="5.5" y1="1" x2="5.5" y2="10" stroke="currentColor" strokeWidth="1.2"/><line x1="8.5" y1="1" x2="8.5" y2="10" stroke="currentColor" strokeWidth="1.2"/><circle cx="2.5" cy="3.5" r="1.3" fill="currentColor"/><circle cx="5.5" cy="6.5" r="1.3" fill="currentColor"/><circle cx="8.5" cy="5" r="1.3" fill="currentColor"/></svg>,
    roadmap: <svg width="11" height="11" viewBox="0 0 11 11"><path d="M1.5 2.5h8M1.5 5.5h6M1.5 8.5h4" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round"/></svg>,
  }
  return (
    <button
      onClick={onClick}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      style={{
        width: 22, height: 22,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        color: active ? hw.purple : hw.textFaint,
        background: active ? hw.purpleDim : 'transparent',
        border: `1px solid ${active ? hw.purple + '40' : 'transparent'}`,
        borderRadius: 2,
      }}
    >
      {icons[icon]}
    </button>
  )
}

function ModeBtn({ label, active, onEnter, onLeave }: {
  label: string; active: boolean; onEnter: () => void; onLeave: () => void
}) {
  return (
    <div
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      style={{
        padding: '2px 6px',
        fontSize: 9, fontWeight: 700,
        color: active ? hw.purple : hw.textFaint,
        background: active ? hw.bgCard : 'transparent',
        cursor: 'default',
        letterSpacing: 0.5,
      }}
    >
      {label}
    </div>
  )
}

function SmallBtn({ label, onClick, onEnter, onLeave }: {
  label: string; onClick: () => void; onEnter: () => void; onLeave: () => void
}) {
  return (
    <button
      onClick={onClick}
      onMouseEnter={e => { e.currentTarget.style.background = hw.bgHover; onEnter() }}
      onMouseLeave={e => { e.currentTarget.style.background = hw.bgCard; onLeave() }}
      style={{
        padding: '2px 8px', fontSize: 10, color: hw.textMuted,
        background: hw.bgCard, border: `1px solid ${hw.border}`,
        borderRadius: 2,
      }}
    >
      {label}
    </button>
  )
}

const tBtn: React.CSSProperties = {
  width: 24, height: 22,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: '#222226',
  border: '1px solid rgba(255,255,255,0.07)',
  borderRadius: 2,
}

const lcdBox: React.CSSProperties = {
  display: 'flex', alignItems: 'center',
  background: '#111114',
  border: '1px solid rgba(255,255,255,0.07)',
  borderRadius: 2,
  height: 22,
  padding: '0 3px',
}

const lcdText: React.CSSProperties = {
  fontFamily: "'Consolas', 'Courier New', monospace",
  fontSize: 12, fontWeight: 700,
  whiteSpace: 'pre',
  letterSpacing: 0,
}
