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
      background: '#1E1E1E',
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Header */}
      <div style={{
        height: 22,
        background: '#1A1A1A',
        borderBottom: '1px solid #333',
        display: 'flex',
        alignItems: 'center',
        padding: '0 8px',
      }}>
        <span style={{ fontSize: 9, fontWeight: 700, color: '#888', letterSpacing: 1 }}>
          MIXER
        </span>
      </div>

      {/* Mixer strips */}
      <div style={{
        flex: 1,
        display: 'flex',
        overflowX: 'auto',
        overflowY: 'hidden',
        padding: '4px 2px',
        gap: 1,
      }}>
        {/* Insert tracks */}
        {allTracks.map((track, idx) => (
          <MixerStrip
            key={track.id}
            name={track.name}
            color={track.color}
            number={idx + 1}
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

        {/* Separator */}
        {allTracks.length > 0 && (
          <div style={{ width: 2, background: '#333', margin: '0 2px', flexShrink: 0 }} />
        )}

        {/* Master strip */}
        <MixerStrip
          name="Master"
          color="#FF4444"
          number={0}
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
            flex: 1, color: '#333', fontSize: 10,
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
  isMaster, onVolumeChange, onPanChange, onToggleMute, onToggleSolo,
}: MixerStripProps) {
  const meterHeight = Math.max(0, Math.min(100, (peakDb + 60) / 72 * 100))
  const meterColor = peakDb > -3 ? '#FF4444' : peakDb > -12 ? '#DDDD44' : '#44AA44'

  return (
    <div style={{
      width: 68,
      minWidth: 68,
      display: 'flex',
      flexDirection: 'column',
      background: isMaster ? '#252525' : '#222',
      borderRadius: 2,
      border: `1px solid ${isMaster ? '#444' : '#2A2A2A'}`,
      flexShrink: 0,
    }}>
      {/* Insert number / name header */}
      <div style={{
        padding: '3px 4px',
        borderBottom: '1px solid #333',
        textAlign: 'center',
      }}>
        <div style={{
          fontSize: 8, color: '#666', marginBottom: 1,
        }}>
          {isMaster ? '' : `Insert ${number}`}
        </div>
        <div style={{
          fontSize: 9, fontWeight: 600,
          color: isMaster ? '#FF6B00' : '#BBB',
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>
          {name}
        </div>
      </div>

      {/* FX slot indicators */}
      <div style={{ padding: '2px 4px', borderBottom: '1px solid #2A2A2A' }}>
        {[0, 1, 2].map(i => (
          <div key={i} style={{
            height: 10,
            background: '#1A1A1A',
            borderRadius: 1,
            marginBottom: 1,
            border: '1px solid #333',
            display: 'flex',
            alignItems: 'center',
            padding: '0 3px',
          }}>
            <span style={{ fontSize: 7, color: '#444' }}>Slot {i + 1}</span>
          </div>
        ))}
      </div>

      {/* Pan knob area */}
      <div style={{
        padding: '4px', display: 'flex', flexDirection: 'column',
        alignItems: 'center', borderBottom: '1px solid #2A2A2A',
      }}>
        <input
          type="range"
          min={-100}
          max={100}
          value={Math.round(pan * 100)}
          onChange={(e) => onPanChange(parseInt(e.target.value) / 100)}
          style={{
            width: 50, height: 8, appearance: 'none',
            background: '#1A1A1A', borderRadius: 4,
            cursor: 'pointer',
          }}
        />
        <span style={{ fontSize: 7, color: '#555', marginTop: 1 }}>
          {pan === 0 ? 'C' : pan < 0 ? `L${Math.round(-pan * 100)}` : `R${Math.round(pan * 100)}`}
        </span>
      </div>

      {/* Fader + meter area */}
      <div style={{
        flex: 1, display: 'flex', padding: '4px 4px',
        gap: 2, minHeight: 80,
      }}>
        {/* LED meter */}
        <div style={{
          width: 8, background: '#111', borderRadius: 1,
          position: 'relative', overflow: 'hidden',
          border: '1px solid #333',
        }}>
          <div style={{
            position: 'absolute', bottom: 0, left: 0, right: 0,
            height: `${meterHeight}%`,
            background: `linear-gradient(to top, ${meterColor}, ${meterColor}88)`,
            transition: 'height 60ms',
          }} />
          {/* dB scale marks */}
          {[0, -6, -12, -24, -48].map(db => {
            const pos = ((db + 60) / 72) * 100
            return (
              <div key={db} style={{
                position: 'absolute', bottom: `${pos}%`, left: 0, right: 0,
                height: 1, background: '#444',
              }} />
            )
          })}
        </div>

        {/* Vertical fader */}
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
              width: 18,
              appearance: 'none',
              background: 'linear-gradient(to right, #2A2A2A, #333, #2A2A2A)',
              borderRadius: 2,
              cursor: 'pointer',
              border: '1px solid #444',
            }}
          />
        </div>
      </div>

      {/* Volume readout */}
      <div style={{
        textAlign: 'center', padding: '2px 0',
        borderTop: '1px solid #2A2A2A',
      }}>
        <span style={{
          fontSize: 8, color: '#777',
          fontFamily: "'Courier New', monospace",
        }}>
          {volumeDb > -60 ? `${volumeDb.toFixed(1)}dB` : '-inf'}
        </span>
      </div>

      {/* M / S buttons */}
      <div style={{
        display: 'flex', gap: 1, padding: '2px 4px 4px',
        justifyContent: 'center',
      }}>
        <button
          onClick={() => onToggleMute?.()}
          style={{
            ...stripBtn,
            color: muted ? '#FF4444' : '#555',
            background: muted ? '#3A2222' : '#1A1A1A',
            borderColor: muted ? '#FF4444' : '#444',
          }}
        >M</button>
        <button
          onClick={onToggleSolo}
          style={{
            ...stripBtn,
            color: soloed ? '#DDDD44' : '#555',
            background: soloed ? '#3A3A22' : '#1A1A1A',
            borderColor: soloed ? '#DDDD44' : '#444',
          }}
        >S</button>
      </div>

      {/* Color strip at bottom */}
      <div style={{ height: 3, background: color, borderRadius: '0 0 2px 2px' }} />
    </div>
  )
}

const stripBtn: React.CSSProperties = {
  width: 22, height: 14,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 8, fontWeight: 800,
  border: '1px solid #444',
  borderRadius: 2,
  padding: 0,
  cursor: 'pointer',
}
