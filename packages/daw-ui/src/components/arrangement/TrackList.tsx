import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'

const ROW_HEIGHT = 18

export function TrackList() {
  const { tracks, selectedTrackId, selectTrack, toggleMute, toggleSolo } = useTrackStore()
  const audioTracks = tracks.filter(t => t.kind !== 'Master')

  return (
    <div style={{
      width: 168, minWidth: 168,
      background: '#08080c',
      borderRight: `1px solid ${hw.border}`,
      display: 'flex', flexDirection: 'column',
    }}>
      <div style={{
        height: 28, background: '#040406', borderBottom: `1px solid ${hw.border}`,
        display: 'flex', alignItems: 'center', padding: '0 10px',
      }}>
        <span style={{
          fontFamily: hw.font.ui, fontSize: 10, fontWeight: 600,
          color: hw.textMuted, letterSpacing: hw.tracking.eyebrow,
          textTransform: 'uppercase',
        }}>Playlist</span>
      </div>

      <div style={{ flex: 1, overflowY: 'auto' }}>
        {audioTracks.map((track, idx) => {
          const selected = selectedTrackId === track.id
          return (
            <div
              key={track.id}
              onClick={() => selectTrack(track.id)}
              style={{
                height: ROW_HEIGHT, display: 'flex', alignItems: 'center',
                borderBottom: `1px solid #0a0a0e`,
                background: selected
                  ? 'rgba(220,38,38,0.10)'
                  : idx % 2 === 0 ? '#08080c' : '#0b0b10',
                cursor: 'default',
                position: 'relative',
                paddingRight: 6,
              }}
            >
              {/* Color stripe */}
              <div style={{ width: 2, height: '100%', background: track.color, flexShrink: 0 }} />

              {/* Track number */}
              <span style={{
                width: 22, textAlign: 'right', flexShrink: 0,
                fontFamily: hw.font.mono, fontSize: 8, fontWeight: 500,
                color: hw.textFaint, letterSpacing: hw.tracking.wide,
                paddingRight: 5,
              }}>{idx + 1}</span>

              {/* Track name */}
              <span style={{
                flex: 1, minWidth: 0,
                fontFamily: hw.font.mono, fontSize: 9, fontWeight: 500,
                color: track.muted ? hw.textFaint : hw.textPrimary,
                letterSpacing: '0.01em',
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>
                {track.name}
              </span>

              {/* Compact M/S LEDs \u2014 circular indicators only when active */}
              <button
                onClick={e => { e.stopPropagation(); toggleMute(track.id) }}
                title={track.muted ? 'Unmute' : 'Mute'}
                style={{
                  width: 11, height: 11, marginLeft: 4, padding: 0, flexShrink: 0,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  border: `1px solid ${track.muted ? hw.red : 'rgba(255,255,255,0.08)'}`,
                  borderRadius: 2,
                  background: track.muted ? hw.red : 'transparent',
                  color: track.muted ? '#000' : hw.textFaint,
                  fontFamily: hw.font.mono, fontSize: 7, fontWeight: 700,
                  cursor: 'default',
                }}
              >M</button>
              <button
                onClick={e => { e.stopPropagation(); toggleSolo(track.id) }}
                title={track.soloed ? 'Unsolo' : 'Solo'}
                style={{
                  width: 11, height: 11, marginLeft: 2, padding: 0, flexShrink: 0,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  border: `1px solid ${track.soloed ? hw.yellow : 'rgba(255,255,255,0.08)'}`,
                  borderRadius: 2,
                  background: track.soloed ? hw.yellow : 'transparent',
                  color: track.soloed ? '#000' : hw.textFaint,
                  fontFamily: hw.font.mono, fontSize: 7, fontWeight: 700,
                  cursor: 'default',
                }}
              >S</button>
            </div>
          )
        })}

        {audioTracks.length === 0 && (
          <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 10 }}>
            Add tracks from toolbar
          </div>
        )}
      </div>
    </div>
  )
}
