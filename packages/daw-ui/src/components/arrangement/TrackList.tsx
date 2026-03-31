import { useTrackStore } from '../../stores/trackStore'

export function TrackList() {
  const { tracks, selectedTrackId, selectTrack, toggleMute, toggleSolo } = useTrackStore()
  const audioTracks = tracks.filter(t => t.kind !== 'Master')

  return (
    <div style={{
      width: 160,
      minWidth: 160,
      background: '#262626',
      borderRight: '1px solid #1A1A1A',
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Header */}
      <div style={{
        height: 22,
        background: '#1E1E1E',
        borderBottom: '1px solid #333',
        display: 'flex',
        alignItems: 'center',
        padding: '0 8px',
      }}>
        <span style={{ fontSize: 9, fontWeight: 700, color: '#888', letterSpacing: 1 }}>PLAYLIST</span>
      </div>

      {/* Track headers */}
      <div style={{ flex: 1, overflowY: 'auto' }}>
        {audioTracks.map((track, idx) => (
          <div
            key={track.id}
            onClick={() => selectTrack(track.id)}
            style={{
              height: 56,
              display: 'flex',
              alignItems: 'stretch',
              borderBottom: '1px solid #1E1E1E',
              background: selectedTrackId === track.id ? '#333' : idx % 2 === 0 ? '#282828' : '#2C2C2C',
              cursor: 'pointer',
            }}
          >
            {/* Color strip */}
            <div style={{ width: 4, background: track.color, flexShrink: 0 }} />

            {/* Track info */}
            <div style={{
              flex: 1, padding: '4px 6px',
              display: 'flex', flexDirection: 'column', justifyContent: 'center',
              minWidth: 0,
            }}>
              {/* Track number + name */}
              <div style={{
                display: 'flex', alignItems: 'center', gap: 4,
              }}>
                <span style={{ fontSize: 8, color: '#555', minWidth: 12 }}>{idx + 1}</span>
                <span style={{
                  fontSize: 10, fontWeight: 600, color: '#CCC',
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                }}>
                  {track.name}
                </span>
              </div>

              {/* Controls row */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 3, marginTop: 4 }}>
                {/* Mute */}
                <button
                  onClick={(e) => { e.stopPropagation(); toggleMute(track.id) }}
                  style={{
                    ...trackBtn,
                    color: track.muted ? '#FF4444' : '#666',
                    background: track.muted ? '#3A2222' : '#222',
                  }}
                >M</button>

                {/* Solo */}
                <button
                  onClick={(e) => { e.stopPropagation(); toggleSolo(track.id) }}
                  style={{
                    ...trackBtn,
                    color: track.soloed ? '#DDDD44' : '#666',
                    background: track.soloed ? '#3A3A22' : '#222',
                  }}
                >S</button>

                {/* Volume indicator */}
                <span style={{ fontSize: 8, color: '#555', marginLeft: 'auto' }}>
                  {track.volume_db > -60 ? `${track.volume_db.toFixed(0)}dB` : '-∞'}
                </span>
              </div>
            </div>
          </div>
        ))}

        {audioTracks.length === 0 && (
          <div style={{ padding: 16, textAlign: 'center', color: '#444', fontSize: 10 }}>
            Add tracks from<br />the toolbar
          </div>
        )}
      </div>
    </div>
  )
}

const trackBtn: React.CSSProperties = {
  width: 18, height: 14,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 8, fontWeight: 800,
  border: '1px solid #444',
  borderRadius: 2,
  padding: 0,
}
