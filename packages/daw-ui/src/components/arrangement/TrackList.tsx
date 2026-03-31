import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'

export function TrackList() {
  const { tracks, selectedTrackId, selectTrack, toggleMute, toggleSolo } = useTrackStore()
  const audioTracks = tracks.filter(t => t.kind !== 'Master')

  return (
    <div style={{
      width: 160,
      minWidth: 160,
      background: hw.bgElevated,
      borderRight: `1px solid ${hw.border}`,
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Header */}
      <div style={{
        height: 22,
        background: hw.bg,
        borderBottom: `1px solid ${hw.border}`,
        display: 'flex',
        alignItems: 'center',
        padding: '0 10px',
      }}>
        <span style={{ fontSize: 11, fontWeight: 500, color: hw.textFaint }}>Playlist</span>
      </div>

      {/* Track headers */}
      <div style={{ flex: 1, overflowY: 'auto' }}>
        {audioTracks.map((track, idx) => {
          const selected = selectedTrackId === track.id
          return (
            <div
              key={track.id}
              onClick={() => selectTrack(track.id)}
              style={{
                height: 56,
                display: 'flex',
                alignItems: 'stretch',
                borderBottom: `1px solid ${hw.border}`,
                background: selected ? hw.bgHover : 'transparent',
                cursor: 'default',
              }}
            >
              {/* Color accent */}
              <div style={{ width: 3, background: track.color, flexShrink: 0 }} />

              {/* Track info */}
              <div style={{
                flex: 1, padding: '6px 8px',
                display: 'flex', flexDirection: 'column', justifyContent: 'center',
              }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                  <span style={{ fontSize: 10, color: hw.textFaint }}>{idx + 1}</span>
                  <span style={{
                    fontSize: 11, color: hw.textSecondary, fontWeight: 500,
                    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  }}>
                    {track.name}
                  </span>
                </div>

                <div style={{ display: 'flex', gap: 3, marginTop: 6 }}>
                  <button
                    onClick={(e) => { e.stopPropagation(); toggleMute(track.id) }}
                    style={{
                      ...tBtn,
                      color: track.muted ? hw.red : hw.textFaint,
                      background: track.muted ? hw.redDim : hw.bgCard,
                      borderColor: track.muted ? hw.red + '30' : hw.border,
                    }}
                  >M</button>
                  <button
                    onClick={(e) => { e.stopPropagation(); toggleSolo(track.id) }}
                    style={{
                      ...tBtn,
                      color: track.soloed ? hw.yellow : hw.textFaint,
                      background: track.soloed ? hw.yellowDim : hw.bgCard,
                      borderColor: track.soloed ? hw.yellow + '30' : hw.border,
                    }}
                  >S</button>
                  <span style={{ fontSize: 9, color: hw.textFaint, marginLeft: 'auto', alignSelf: 'center' }}>
                    {track.volume_db > -60 ? `${track.volume_db.toFixed(0)} dB` : '-\u221E'}
                  </span>
                </div>
              </div>
            </div>
          )
        })}

        {audioTracks.length === 0 && (
          <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 11 }}>
            Add tracks from toolbar
          </div>
        )}
      </div>
    </div>
  )
}

const tBtn: React.CSSProperties = {
  width: 20, height: 16,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 9, fontWeight: 600,
  border: '1px solid',
  borderRadius: 3,
  padding: 0,
}
