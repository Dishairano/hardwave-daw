import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'

export function TrackList() {
  const { tracks, selectedTrackId, selectTrack, toggleMute, toggleSolo } = useTrackStore()
  const audioTracks = tracks.filter(t => t.kind !== 'Master')

  return (
    <div style={{
      width: 155, minWidth: 155,
      background: '#3C3C3C',
      borderRight: '1px solid rgba(0,0,0,0.4)',
      display: 'flex', flexDirection: 'column',
    }}>
      <div style={{
        height: 22, background: '#333', borderBottom: '1px solid rgba(0,0,0,0.4)',
        display: 'flex', alignItems: 'center', padding: '0 8px',
      }}>
        <span style={{ fontSize: 10, color: '#999' }}>Playlist</span>
      </div>

      <div style={{ flex: 1, overflowY: 'auto' }}>
        {audioTracks.map((track, idx) => (
          <div
            key={track.id}
            onClick={() => selectTrack(track.id)}
            style={{
              height: 56, display: 'flex', alignItems: 'stretch',
              borderBottom: '1px solid rgba(0,0,0,0.2)',
              background: selectedTrackId === track.id ? '#4A4A4A' : idx % 2 === 0 ? '#3C3C3C' : '#404040',
              cursor: 'default',
            }}
          >
            <div style={{ width: 3, background: track.color, flexShrink: 0 }} />
            <div style={{ flex: 1, padding: '5px 7px', display: 'flex', flexDirection: 'column', justifyContent: 'center' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                <span style={{ fontSize: 9, color: '#888' }}>{idx + 1}</span>
                <span style={{
                  fontSize: 10, color: '#CCC', fontWeight: 500,
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                }}>
                  {track.name}
                </span>
              </div>
              <div style={{ display: 'flex', gap: 3, marginTop: 5 }}>
                <button
                  onClick={e => { e.stopPropagation(); toggleMute(track.id) }}
                  style={{
                    ...tb,
                    color: track.muted ? '#CC3333' : '#999',
                    background: track.muted ? 'rgba(204,51,51,0.15)' : '#333',
                  }}
                >M</button>
                <button
                  onClick={e => { e.stopPropagation(); toggleSolo(track.id) }}
                  style={{
                    ...tb,
                    color: track.soloed ? '#DDAA00' : '#999',
                    background: track.soloed ? 'rgba(221,170,0,0.15)' : '#333',
                  }}
                >S</button>
                <span style={{ fontSize: 8, color: '#888', marginLeft: 'auto', alignSelf: 'center' }}>
                  {track.volume_db > -60 ? `${track.volume_db.toFixed(0)}dB` : '-\u221E'}
                </span>
              </div>
            </div>
          </div>
        ))}

        {audioTracks.length === 0 && (
          <div style={{ padding: 16, textAlign: 'center', color: '#777', fontSize: 10 }}>
            Add tracks from toolbar
          </div>
        )}
      </div>
    </div>
  )
}

const tb: React.CSSProperties = {
  width: 18, height: 14,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 8, fontWeight: 700,
  border: '1px solid rgba(0,0,0,0.3)',
  borderRadius: 2, padding: 0,
}
