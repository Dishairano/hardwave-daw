import { useTransportStore } from '../../stores/transportStore'
import { useTrackStore } from '../../stores/trackStore'

export function Toolbar() {
  const { playing, bpm, positionSamples, sampleRate, togglePlayback, stop, setBpm } = useTransportStore()
  const { tracks, selectedTrackId, addAudioTrack, addMidiTrack, importAudioFile } = useTrackStore()

  const seconds = sampleRate > 0 ? positionSamples / sampleRate : 0
  const mins = Math.floor(seconds / 60)
  const secs = Math.floor(seconds % 60)
  const ms = Math.floor((seconds % 1) * 1000)
  const timeStr = `${mins}:${String(secs).padStart(2, '0')}.${String(ms).padStart(3, '0')}`

  const handleImport = async () => {
    // Use Tauri file dialog
    const { open } = await import('@tauri-apps/plugin-dialog')
    const selected = await open({
      multiple: false,
      filters: [{ name: 'Audio', extensions: ['wav', 'flac', 'mp3', 'ogg', 'aac', 'm4a'] }],
    })
    if (!selected) return

    // Find or create a track to put it on
    let trackId = selectedTrackId
    const audioTracks = tracks.filter(t => t.kind === 'Audio')
    if (!trackId || !audioTracks.find(t => t.id === trackId)) {
      if (audioTracks.length === 0) {
        await addAudioTrack()
        // Re-read from store after adding
        const { tracks: updated } = useTrackStore.getState()
        const newTrack = updated.find(t => t.kind === 'Audio')
        if (!newTrack) return
        trackId = newTrack.id
      } else {
        trackId = audioTracks[0].id
      }
    }

    await importAudioFile(trackId, selected as string)
  }

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      gap: 12,
      padding: '0 16px',
      height: 42,
      background: '#111113',
      borderBottom: '1px solid rgba(255,255,255,0.06)',
    }}>
      {/* Transport buttons */}
      <div style={{ display: 'flex', gap: 4 }}>
        <button onClick={stop} style={btnStyle}>
          <svg width="12" height="12" viewBox="0 0 12 12"><rect x="2" y="2" width="8" height="8" fill="currentColor" /></svg>
        </button>
        <button onClick={togglePlayback} style={{ ...btnStyle, color: playing ? '#22c55e' : '#888' }}>
          <svg width="12" height="12" viewBox="0 0 12 12"><polygon points="2,0 12,6 2,12" fill="currentColor" /></svg>
        </button>
      </div>

      {/* Time display */}
      <div style={{ fontFamily: 'monospace', fontSize: 16, fontWeight: 700, color: '#fff', minWidth: 120, textAlign: 'center' }}>
        {timeStr}
      </div>

      {/* BPM */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
        <input
          type="number"
          value={bpm}
          onChange={(e) => setBpm(parseFloat(e.target.value) || 140)}
          style={{
            width: 55, padding: '3px 6px', background: '#1a1a1c',
            border: '1px solid rgba(255,255,255,0.08)', borderRadius: 4,
            color: '#fff', fontSize: 12, fontFamily: 'monospace', textAlign: 'center',
          }}
        />
        <span style={{ fontSize: 10, color: '#555' }}>BPM</span>
      </div>

      <div style={{ flex: 1 }} />

      {/* Import audio */}
      <button onClick={handleImport} style={{ ...addBtnStyle, color: '#7c3aed' }}>
        Import Audio
      </button>

      {/* Add track buttons */}
      <button onClick={() => addAudioTrack()} style={addBtnStyle}>+ Audio</button>
      <button onClick={() => addMidiTrack()} style={addBtnStyle}>+ MIDI</button>
    </div>
  )
}

const btnStyle: React.CSSProperties = {
  width: 28, height: 28, display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)',
  borderRadius: 6, color: '#888', cursor: 'pointer', fontSize: 12,
}

const addBtnStyle: React.CSSProperties = {
  padding: '4px 10px', background: 'rgba(255,255,255,0.04)',
  border: '1px solid rgba(255,255,255,0.08)', borderRadius: 6,
  color: '#888', cursor: 'pointer', fontSize: 11, fontFamily: 'inherit',
}
