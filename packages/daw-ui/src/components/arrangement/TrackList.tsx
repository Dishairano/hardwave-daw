import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'

export function TrackList() {
  const { tracks, selectedTrackId, selectTrack, toggleMute, toggleSolo } = useTrackStore()
  const audioTracks = tracks.filter(t => t.kind !== 'Master')

  return (
    <div style={{
      width: 155, minWidth: 155,
      background: 'rgba(255,255,255,0.02)',
      backdropFilter: hw.blur.sm,
      borderRight: `1px solid ${hw.border}`,
      display: 'flex', flexDirection: 'column',
    }}>
      <div style={{
        height: 22, background: 'rgba(255,255,255,0.01)', borderBottom: `1px solid ${hw.border}`,
        display: 'flex', alignItems: 'center', padding: '0 8px',
      }}>
        <span style={{ fontSize: 10, color: hw.textMuted }}>Playlist</span>
      </div>

      <div style={{ flex: 1, overflowY: 'auto' }}>
        {audioTracks.map((track, idx) => (
          <div
            key={track.id}
            onClick={() => selectTrack(track.id)}
            style={{
              height: 56, display: 'flex', alignItems: 'stretch',
              borderBottom: `1px solid ${hw.border}`,
              background: selectedTrackId === track.id
                ? hw.accentDim
                : idx % 2 === 0 ? 'transparent' : 'rgba(255,255,255,0.015)',
              cursor: 'default',
              transition: 'background 0.1s',
            }}
          >
            <div style={{ width: 3, background: track.color, flexShrink: 0 }} />
            <div style={{ flex: 1, padding: '5px 7px', display: 'flex', flexDirection: 'column', justifyContent: 'center' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                <span style={{ fontSize: 9, color: hw.textFaint }}>{idx + 1}</span>
                <span style={{
                  fontSize: 10, color: hw.textPrimary, fontWeight: 500,
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
                    color: track.muted ? hw.red : hw.textMuted,
                    background: track.muted ? hw.redDim : 'rgba(255,255,255,0.04)',
                  }}
                >M</button>
                <button
                  onClick={e => { e.stopPropagation(); toggleSolo(track.id) }}
                  style={{
                    ...tb,
                    color: track.soloed ? hw.yellow : hw.textMuted,
                    background: track.soloed ? hw.yellowDim : 'rgba(255,255,255,0.04)',
                  }}
                >S</button>
                <span style={{ fontSize: 8, color: hw.textFaint, marginLeft: 'auto', alignSelf: 'center' }}>
                  {track.volume_db > -60 ? `${track.volume_db.toFixed(0)}dB` : '-\u221E'}
                </span>
              </div>
            </div>
          </div>
        ))}

        {audioTracks.length === 0 && (
          <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 10 }}>
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
  border: `1px solid rgba(255,255,255,0.06)`,
  borderRadius: 6, padding: 0,
}
