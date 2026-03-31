import { useTrackStore } from '../../stores/trackStore'
import { useMeterStore } from '../../stores/meterStore'
import { useEffect } from 'react'

export function MixerPanel() {
  const { tracks, setVolume, setPan, toggleMute, toggleSolo } = useTrackStore()
  const { master, startListening } = useMeterStore()

  useEffect(() => { startListening() }, [])

  const allTracks = tracks.filter(t => t.kind !== 'Master')
  const masterTrack = tracks.find(t => t.kind === 'Master')

  return (
    <div style={{
      height: '100%',
      background: '#191919',
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Header */}
      <div style={{
        height: 20,
        background: '#252525',
        borderBottom: '1px solid #111',
        display: 'flex',
        alignItems: 'center',
        padding: '0 6px',
      }}>
        <span style={{ fontSize: 10, color: '#777' }}>Mixer</span>
      </div>

      {/* Mixer strips — FL horizontal scroll */}
      <div style={{
        flex: 1,
        display: 'flex',
        overflowX: 'auto',
        overflowY: 'hidden',
        padding: '2px 1px',
        gap: 0,
      }}>
        {/* Insert tracks */}
        {allTracks.map((track, idx) => (
          <MixerStrip
            key={track.id}
            name={track.name}
            color={track.color}
            number={idx}
            volumeDb={track.volume_db}
            pan={track.pan}
            muted={track.muted}
            soloed={track.soloed}
            peakDb={-60}
            onVolumeChange={(db) => setVolume(track.id, db)}
            onPanChange={(p) => setPan(track.id, p)}
            onToggleMute={() => toggleMute(track.id)}
            onToggleSolo={() => toggleSolo(track.id)}
          />
        ))}

        {/* Separator before master */}
        {allTracks.length > 0 && (
          <div style={{ width: 1, background: '#333', margin: '4px 1px', flexShrink: 0 }} />
        )}

        {/* Master strip */}
        <MixerStrip
          name="Master"
          color="#888"
          number={-1}
          volumeDb={masterTrack?.volume_db ?? 0}
          pan={0}
          muted={masterTrack?.muted ?? false}
          soloed={false}
          peakDb={master.peak_db}
          isMaster
          onVolumeChange={masterTrack ? (db) => setVolume(masterTrack.id, db) : undefined}
          onPanChange={() => {}}
          onToggleMute={masterTrack ? () => toggleMute(masterTrack.id) : undefined}
          onToggleSolo={() => {}}
        />

        {allTracks.length === 0 && (
          <div style={{
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            flex: 1, color: '#282828', fontSize: 10,
          }}>
            Add tracks to see mixer inserts
          </div>
        )}
      </div>
    </div>
  )
}

interface MixerStripProps {
  name: string
  color: string
  number: number
  volumeDb: number
  pan: number
  muted: boolean
  soloed: boolean
  peakDb: number
  isMaster?: boolean
  onVolumeChange?: (db: number) => void
  onPanChange: (pan: number) => void
  onToggleMute?: () => void
  onToggleSolo: () => void
}

function MixerStrip({
  name, color, number, volumeDb, pan, muted, soloed, peakDb,
  isMaster, onVolumeChange, onToggleMute, onToggleSolo,
}: MixerStripProps) {
  // FL fader color: grey default, green selected
  const meterHeight = Math.max(0, Math.min(100, (peakDb + 60) / 72 * 100))

  return (
    <div style={{
      width: 58,
      minWidth: 58,
      display: 'flex',
      flexDirection: 'column',
      background: '#1D1D1D',
      border: '1px solid #161616',
      flexShrink: 0,
    }}>
      {/* Track number */}
      <div style={{
        height: 16, display: 'flex', alignItems: 'center', justifyContent: 'center',
        borderBottom: '1px solid #222',
        fontSize: 9, color: '#555',
      }}>
        {isMaster ? 'M' : `${number}`}
      </div>

      {/* Track name */}
      <div style={{
        height: 18, display: 'flex', alignItems: 'center', justifyContent: 'center',
        borderBottom: '1px solid #222',
        padding: '0 2px',
      }}>
        <span style={{
          fontSize: 8, color: '#999',
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>
          {name}
        </span>
      </div>

      {/* FX slot indicators — FL has 10, we show 3 */}
      <div style={{ padding: '1px 2px', borderBottom: '1px solid #1A1A1A' }}>
        {[0, 1, 2].map(i => (
          <div key={i} style={{
            height: 9,
            background: '#161616',
            border: '1px solid #222',
            marginBottom: 1,
            display: 'flex', alignItems: 'center', padding: '0 2px',
          }}>
            <span style={{ fontSize: 6, color: '#333' }}>{i + 1}</span>
          </div>
        ))}
      </div>

      {/* Fader + meter area */}
      <div style={{
        flex: 1, display: 'flex', padding: '2px',
        gap: 1, minHeight: 70,
      }}>
        {/* FL-style dual peak meter (L/R) */}
        <div style={{ display: 'flex', gap: 0, width: 10, flexShrink: 0 }}>
          {[0, 1].map(ch => (
            <div key={ch} style={{
              flex: 1, background: '#0A0A0A', position: 'relative',
              border: '1px solid #1A1A1A',
            }}>
              <div style={{
                position: 'absolute', bottom: 0, left: 0, right: 0,
                height: `${meterHeight}%`,
                background: peakDb > -3
                  ? 'linear-gradient(to top, #5A5, #CC4, #C44)'
                  : peakDb > -12
                  ? 'linear-gradient(to top, #5A5, #CC4)'
                  : '#5A5',
                transition: 'height 60ms',
              }} />
            </div>
          ))}
        </div>

        {/* Vertical fader — FL grey fader with groove */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
          <input
            type="range"
            min={-60}
            max={12}
            step={0.1}
            value={volumeDb}
            onChange={(e) => onVolumeChange?.(parseFloat(e.target.value))}
            style={{
              writingMode: 'vertical-lr' as any,
              direction: 'rtl',
              flex: 1,
              width: 14,
              appearance: 'none',
              background: 'linear-gradient(to right, #1A1A1A, #222, #1A1A1A)',
              border: '1px solid #333',
              cursor: 'default',
            }}
          />
        </div>
      </div>

      {/* dB readout */}
      <div style={{
        textAlign: 'center', padding: '1px 0',
        borderTop: '1px solid #1A1A1A',
      }}>
        <span style={{
          fontSize: 8, color: '#666',
          fontFamily: "'Consolas', monospace",
        }}>
          {volumeDb > -60 ? `${volumeDb.toFixed(1)}` : '-inf'}
        </span>
      </div>

      {/* M/S row */}
      <div style={{
        display: 'flex', gap: 0, padding: '1px 2px 2px',
        justifyContent: 'center',
      }}>
        <button
          onClick={() => onToggleMute?.()}
          style={{
            ...stripBtn,
            color: muted ? '#C44' : '#444',
            background: muted ? '#2A1818' : '#161616',
          }}
        >M</button>
        <button
          onClick={onToggleSolo}
          style={{
            ...stripBtn,
            color: soloed ? '#CC4' : '#444',
            background: soloed ? '#2A2A18' : '#161616',
          }}
        >S</button>
      </div>

      {/* Color strip at bottom */}
      <div style={{ height: 2, background: color }} />
    </div>
  )
}

const stripBtn: React.CSSProperties = {
  width: 20, height: 13,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 7, fontWeight: 700,
  border: '1px solid #2A2A2A',
  padding: 0,
}
