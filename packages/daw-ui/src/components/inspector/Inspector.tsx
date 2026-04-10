import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'

export function Inspector() {
  const { tracks, selectedTrackId, setVolume, setPan, removeTrack } = useTrackStore()
  const track = tracks.find(t => t.id === selectedTrackId)

  if (!track) {
    return (
      <div style={{ padding: 16, color: hw.textFaint, fontSize: 11, textAlign: 'center' }}>
        Select a track to inspect
      </div>
    )
  }

  return (
    <div style={{ padding: 12, display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div style={{ fontSize: 12, fontWeight: 700, color: hw.textPrimary }}>{track.name}</div>
      <div style={{ fontSize: 9, color: hw.textFaint }}>{track.kind} Track</div>

      <label style={labelStyle}>
        Volume
        <input type="range" min={-60} max={12} step={0.1} value={track.volume_db}
          onChange={(e) => setVolume(track.id, parseFloat(e.target.value))}
          style={{ width: '100%' }}
        />
        <span style={{ fontSize: 9, color: hw.textFaint }}>{track.volume_db.toFixed(1)} dB</span>
      </label>

      <label style={labelStyle}>
        Pan
        <input type="range" min={-1} max={1} step={0.01} value={track.pan}
          onChange={(e) => setPan(track.id, parseFloat(e.target.value))}
          style={{ width: '100%' }}
        />
      </label>

      <div style={{ fontSize: 10, color: hw.textFaint, marginTop: 8 }}>
        Inserts: {track.insert_count}
      </div>

      {track.kind !== 'Master' && (
        <button
          onClick={() => removeTrack(track.id)}
          style={{
            marginTop: 'auto', padding: '5px', fontSize: 10,
            background: hw.redDim, border: `1px solid rgba(239,68,68,0.2)`,
            borderRadius: hw.radius.md, color: hw.red, cursor: 'pointer', fontFamily: 'inherit',
          }}
        >
          Remove Track
        </button>
      )}
    </div>
  )
}

const labelStyle: React.CSSProperties = {
  display: 'flex', flexDirection: 'column', gap: 2, fontSize: 10, color: hw.textMuted,
}
