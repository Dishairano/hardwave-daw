import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'
import { useTransportStore } from '../../stores/transportStore'

const CLIP_PALETTE = [
  '#DC2626', '#10B981', '#A855F7', '#F59E0B',
  '#3B82F6', '#EC4899', '#06B6D4', '#84CC16',
]

export function Inspector() {
  const { tracks, selectedTrackId, selectedClipId, setVolume, setPan, removeTrack } = useTrackStore()
  const { clipColorOverrides, setClipColor } = useTransportStore()
  const track = tracks.find(t => t.id === selectedTrackId)
  const selectedClip = selectedClipId
    ? tracks.flatMap(t => t.clips).find(c => c.id === selectedClipId) || null
    : null

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

      {selectedClip && (
        <div style={{ marginTop: 10, paddingTop: 10, borderTop: `1px solid ${hw.border}`, display: 'flex', flexDirection: 'column', gap: 6 }}>
          <div style={{ fontSize: 10, fontWeight: 700, color: hw.textSecondary }}>Clip: {selectedClip.name}</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>Color</div>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
            {CLIP_PALETTE.map(c => {
              const active = clipColorOverrides[selectedClip.id] === c
              return (
                <button
                  key={c}
                  onClick={() => setClipColor(selectedClip.id, c)}
                  title={c}
                  style={{
                    width: 18, height: 18, borderRadius: 4, background: c,
                    border: active ? `2px solid #fff` : `1px solid ${hw.borderDark}`,
                    padding: 0, cursor: 'pointer',
                  }}
                />
              )
            })}
            <button
              onClick={() => setClipColor(selectedClip.id, null)}
              title="Reset to default"
              style={{
                fontSize: 9, padding: '0 6px', height: 18,
                background: 'transparent', color: hw.textFaint,
                border: `1px solid ${hw.borderDark}`, borderRadius: 4, cursor: 'pointer',
              }}
            >
              reset
            </button>
          </div>
        </div>
      )}

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
