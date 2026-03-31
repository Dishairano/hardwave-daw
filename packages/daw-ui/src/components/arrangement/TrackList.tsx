import { useTrackStore } from '../../stores/trackStore'

export function TrackList() {
  const { tracks, selectedTrackId, selectTrack, toggleMute, toggleSolo } = useTrackStore()
  const audioTracks = tracks.filter(t => t.kind !== 'Master')

  return (
    <div style={{
      width: 150,
      minWidth: 150,
      background: '#1D1D1D',
      borderRight: '1px solid #111',
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Header — matches ruler height in Arrangement */}
      <div style={{
        height: 22,
        background: '#252525',
        borderBottom: '1px solid #111',
        display: 'flex',
        alignItems: 'center',
        padding: '0 6px',
      }}>
        <span style={{ fontSize: 9, color: '#666' }}>Playlist</span>
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
              borderBottom: '1px solid #151515',
              background: selectedTrackId === track.id ? '#2A2A2A' : idx % 2 === 0 ? '#1C1C1C' : '#1E1E1E',
              cursor: 'default',
            }}
          >
            {/* Color strip — FL has thin color bar on left */}
            <div style={{ width: 3, background: track.color, flexShrink: 0 }} />

            {/* Track info */}
            <div style={{
              flex: 1, padding: '4px 5px',
              display: 'flex', flexDirection: 'column', justifyContent: 'center',
              minWidth: 0,
            }}>
              {/* Track number + name */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                <span style={{ fontSize: 8, color: '#444', minWidth: 10 }}>{idx + 1}</span>
                <span style={{
                  fontSize: 10, color: '#B0B0B0',
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                }}>
                  {track.name}
                </span>
              </div>

              {/* M/S buttons */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 2, marginTop: 4 }}>
                <button
                  onClick={(e) => { e.stopPropagation(); toggleMute(track.id) }}
                  style={{
                    ...trackBtn,
                    color: track.muted ? '#C44' : '#555',
                    background: track.muted ? '#2A1818' : '#1A1A1A',
                  }}
                >M</button>
                <button
                  onClick={(e) => { e.stopPropagation(); toggleSolo(track.id) }}
                  style={{
                    ...trackBtn,
                    color: track.soloed ? '#CC4' : '#555',
                    background: track.soloed ? '#2A2A18' : '#1A1A1A',
                  }}
                >S</button>
                <span style={{ fontSize: 8, color: '#444', marginLeft: 'auto' }}>
                  {track.volume_db > -60 ? `${track.volume_db.toFixed(0)}` : '-\u221E'}
                </span>
              </div>
            </div>
          </div>
        ))}

        {audioTracks.length === 0 && (
          <div style={{ padding: 12, textAlign: 'center', color: '#333', fontSize: 10 }}>
            Add tracks from<br />the toolbar
          </div>
        )}
      </div>
    </div>
  )
}

const trackBtn: React.CSSProperties = {
  width: 16, height: 13,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 8, fontWeight: 700,
  border: '1px solid #333',
  padding: 0,
}
