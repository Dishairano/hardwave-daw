import { useTrackStore } from '../../stores/trackStore'

export function TrackList() {
  const { tracks, selectedTrackId, selectTrack, toggleMute, toggleSolo, removeTrack } = useTrackStore()

  return (
    <div style={{
      background: '#0e0e10',
      borderRight: '1px solid rgba(255,255,255,0.06)',
      overflowY: 'auto',
    }}>
      <div style={{ padding: '8px 0' }}>
        {tracks.map((track) => (
          <div
            key={track.id}
            onClick={() => selectTrack(track.id)}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 6,
              padding: '6px 10px',
              cursor: 'pointer',
              background: selectedTrackId === track.id ? 'rgba(255,255,255,0.04)' : 'transparent',
              borderLeft: `3px solid ${track.color}`,
            }}
          >
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 11, fontWeight: 600, color: '#ddd', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {track.name}
              </div>
              <div style={{ fontSize: 9, color: '#555' }}>{track.kind}</div>
            </div>

            <div style={{ display: 'flex', gap: 2 }}>
              <button
                onClick={(e) => { e.stopPropagation(); toggleMute(track.id) }}
                style={{
                  ...smallBtn,
                  color: track.muted ? '#ef4444' : '#555',
                  background: track.muted ? 'rgba(239,68,68,0.15)' : 'transparent',
                }}
              >M</button>
              <button
                onClick={(e) => { e.stopPropagation(); toggleSolo(track.id) }}
                style={{
                  ...smallBtn,
                  color: track.soloed ? '#eab308' : '#555',
                  background: track.soloed ? 'rgba(234,179,8,0.15)' : 'transparent',
                }}
              >S</button>
            </div>
          </div>
        ))}
      </div>

      {tracks.length === 0 && (
        <div style={{ padding: 20, textAlign: 'center', color: '#333', fontSize: 11 }}>
          No tracks yet.<br />Use the toolbar to add tracks.
        </div>
      )}
    </div>
  )
}

const smallBtn: React.CSSProperties = {
  width: 18, height: 18, display: 'flex', alignItems: 'center', justifyContent: 'center',
  border: '1px solid rgba(255,255,255,0.08)', borderRadius: 3,
  fontSize: 9, fontWeight: 700, cursor: 'pointer', background: 'transparent',
  fontFamily: 'inherit',
}
