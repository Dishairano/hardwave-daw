import { useTrackStore } from '../../stores/trackStore'
import { useMeterStore } from '../../stores/meterStore'
import { useEffect } from 'react'

export function MixerPanel() {
  const { tracks, setVolume, setPan, toggleMute, toggleSolo } = useTrackStore()
  const { master, startListening } = useMeterStore()

  useEffect(() => { startListening() }, [])

  return (
    <div style={{
      display: 'flex',
      gap: 1,
      background: '#0e0e10',
      borderTop: '1px solid rgba(255,255,255,0.06)',
      overflowX: 'auto',
      padding: '8px',
    }}>
      {tracks.map((track) => (
        <div key={track.id} style={{
          width: 80,
          minWidth: 80,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          gap: 4,
          padding: '6px',
          background: 'rgba(255,255,255,0.02)',
          borderRadius: 6,
          border: '1px solid rgba(255,255,255,0.04)',
        }}>
          {/* Track name */}
          <div style={{
            fontSize: 9, fontWeight: 600, color: '#aaa',
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
            width: '100%', textAlign: 'center',
          }}>
            {track.name}
          </div>

          {/* Color indicator */}
          <div style={{ width: '100%', height: 2, borderRadius: 1, background: track.color }} />

          {/* Fader */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', width: '100%' }}>
            <input
              type="range"
              min={-60}
              max={12}
              step={0.1}
              value={track.volume_db}
              onChange={(e) => setVolume(track.id, parseFloat(e.target.value))}
              style={{
                writingMode: 'vertical-lr' as any,
                direction: 'rtl',
                height: 80,
                width: 20,
                appearance: 'none',
                background: 'rgba(255,255,255,0.06)',
                borderRadius: 4,
                cursor: 'pointer',
              }}
            />
            <span style={{ fontSize: 9, color: '#666', marginTop: 2 }}>
              {track.volume_db > -60 ? `${track.volume_db.toFixed(1)}` : '-inf'}
            </span>
          </div>

          {/* Pan knob (simplified as text) */}
          <div style={{ fontSize: 9, color: '#555' }}>
            {track.pan === 0 ? 'C' : track.pan < 0 ? `L${Math.round(-track.pan * 100)}` : `R${Math.round(track.pan * 100)}`}
          </div>

          {/* M/S buttons */}
          <div style={{ display: 'flex', gap: 2 }}>
            <button
              onClick={() => toggleMute(track.id)}
              style={{
                ...mixBtnStyle,
                color: track.muted ? '#ef4444' : '#555',
                background: track.muted ? 'rgba(239,68,68,0.15)' : 'rgba(255,255,255,0.04)',
              }}
            >M</button>
            <button
              onClick={() => toggleSolo(track.id)}
              style={{
                ...mixBtnStyle,
                color: track.soloed ? '#eab308' : '#555',
                background: track.soloed ? 'rgba(234,179,8,0.15)' : 'rgba(255,255,255,0.04)',
              }}
            >S</button>
          </div>
        </div>
      ))}

      {/* Master meter */}
      <div style={{
        width: 80, minWidth: 80,
        display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 4,
        padding: '6px', background: 'rgba(239,68,68,0.04)', borderRadius: 6,
        border: '1px solid rgba(239,68,68,0.1)',
      }}>
        <div style={{ fontSize: 9, fontWeight: 600, color: '#ef4444' }}>MASTER</div>
        <div style={{ width: '100%', height: 2, borderRadius: 1, background: '#ef4444' }} />

        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 2, width: '100%' }}>
          {/* Peak meter bars */}
          <div style={{ width: '100%', height: 80, background: 'rgba(0,0,0,0.3)', borderRadius: 3, position: 'relative', overflow: 'hidden' }}>
            <div style={{
              position: 'absolute', bottom: 0, left: 0, right: 0,
              height: `${Math.max(0, (master.peak_db + 60) / 72 * 100)}%`,
              background: master.clipped ? '#ef4444' : master.peak_db > -6 ? '#eab308' : '#22c55e',
              borderRadius: '0 0 3px 3px',
              transition: 'height 50ms',
            }} />
          </div>

          <span style={{ fontSize: 9, color: '#666' }}>
            {master.peak_db > -100 ? `${master.peak_db.toFixed(1)}` : '-inf'}
          </span>
        </div>

        <div style={{ fontSize: 8, color: '#555' }}>
          {master.lufs_m !== null ? `${master.lufs_m.toFixed(1)} LUFS` : '— LUFS'}
        </div>
      </div>

      {tracks.length === 0 && (
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', flex: 1, color: '#222', fontSize: 11 }}>
          Mixer — add tracks to see channel strips
        </div>
      )}
    </div>
  )
}

const mixBtnStyle: React.CSSProperties = {
  width: 22, height: 18, display: 'flex', alignItems: 'center', justifyContent: 'center',
  border: '1px solid rgba(255,255,255,0.08)', borderRadius: 3,
  fontSize: 9, fontWeight: 700, cursor: 'pointer', fontFamily: 'inherit',
}
