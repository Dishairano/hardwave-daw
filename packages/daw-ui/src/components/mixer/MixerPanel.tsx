import { hw } from '../../theme'
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
      background: hw.bgElevated,
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Header */}
      <div style={{
        height: 28,
        background: hw.bg,
        borderBottom: `1px solid ${hw.border}`,
        display: 'flex',
        alignItems: 'center',
        padding: '0 10px',
      }}>
        <span style={{ fontSize: 11, fontWeight: 600, color: hw.textSecondary }}>Mixer</span>
      </div>

      {/* Strips */}
      <div style={{
        flex: 1, display: 'flex',
        overflowX: 'auto', overflowY: 'hidden',
        padding: 4, gap: 3,
      }}>
        {allTracks.map((track, idx) => (
          <Strip
            key={track.id}
            name={track.name}
            color={track.color}
            number={idx}
            volumeDb={track.volume_db}
            pan={track.pan}
            muted={track.muted}
            soloed={track.soloed}
            peakDb={-60}
            onVolume={(db) => setVolume(track.id, db)}
            onPan={(p) => setPan(track.id, p)}
            onMute={() => toggleMute(track.id)}
            onSolo={() => toggleSolo(track.id)}
          />
        ))}

        {allTracks.length > 0 && (
          <div style={{ width: 1, background: hw.border, margin: '0 2px', flexShrink: 0 }} />
        )}

        {/* Master */}
        <Strip
          name="Master"
          color={hw.red}
          number={-1}
          volumeDb={masterTrack?.volume_db ?? 0}
          pan={0}
          muted={masterTrack?.muted ?? false}
          soloed={false}
          peakDb={master.peak_db}
          isMaster
          onVolume={masterTrack ? (db) => setVolume(masterTrack.id, db) : undefined}
          onPan={() => {}}
          onMute={masterTrack ? () => toggleMute(masterTrack.id) : undefined}
          onSolo={() => {}}
        />

        {allTracks.length === 0 && (
          <div style={{
            flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
            color: hw.textFaint, fontSize: 11,
          }}>
            Add tracks to see mixer
          </div>
        )}
      </div>
    </div>
  )
}

interface StripProps {
  name: string; color: string; number: number
  volumeDb: number; pan: number; muted: boolean; soloed: boolean; peakDb: number
  isMaster?: boolean
  onVolume?: (db: number) => void
  onPan: (p: number) => void
  onMute?: () => void
  onSolo: () => void
}

function Strip({
  name, color, number, volumeDb, pan, muted, soloed, peakDb,
  isMaster, onVolume, onMute, onSolo,
}: StripProps) {
  const meterH = Math.max(0, Math.min(100, (peakDb + 60) / 72 * 100))
  const meterColor = peakDb > -3 ? hw.red : peakDb > -12 ? hw.yellow : hw.green

  return (
    <div style={{
      width: 64, minWidth: 64,
      display: 'flex', flexDirection: 'column',
      background: hw.bgCard,
      border: `1px solid ${hw.border}`,
      borderRadius: hw.radius.md,
      flexShrink: 0,
      overflow: 'hidden',
    }}>
      {/* Color top bar */}
      <div style={{ height: 2, background: color }} />

      {/* Name */}
      <div style={{
        padding: '4px 4px 2px',
        textAlign: 'center',
        borderBottom: `1px solid ${hw.border}`,
      }}>
        <div style={{ fontSize: 8, color: hw.textFaint }}>
          {isMaster ? 'Master' : number}
        </div>
        <div style={{
          fontSize: 9, fontWeight: 600, color: hw.textSecondary,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>
          {name}
        </div>
      </div>

      {/* Fader + meter */}
      <div style={{ flex: 1, display: 'flex', padding: 3, gap: 2, minHeight: 70 }}>
        {/* Meter */}
        <div style={{
          width: 6, background: hw.bg, borderRadius: 2,
          position: 'relative', overflow: 'hidden',
        }}>
          <div style={{
            position: 'absolute', bottom: 0, left: 0, right: 0,
            height: `${meterH}%`, background: meterColor,
            borderRadius: 2,
            transition: 'height 60ms',
            boxShadow: meterH > 50 ? `0 0 4px ${meterColor}40` : 'none',
          }} />
        </div>

        {/* Fader */}
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <input
            type="range"
            min={-60} max={12} step={0.1}
            value={volumeDb}
            onChange={(e) => onVolume?.(parseFloat(e.target.value))}
            style={{
              writingMode: 'vertical-lr' as any,
              direction: 'rtl',
              flex: 1, width: 14,
              appearance: 'none',
              background: `linear-gradient(to bottom, ${hw.bgCard}, ${hw.bg})`,
              borderRadius: 3,
              cursor: 'pointer',
            }}
          />
        </div>
      </div>

      {/* dB readout */}
      <div style={{ textAlign: 'center', padding: '2px 0', borderTop: `1px solid ${hw.border}` }}>
        <span style={{
          fontSize: 9, color: hw.textFaint,
          fontFamily: "'SF Mono', 'Consolas', monospace",
        }}>
          {volumeDb > -60 ? `${volumeDb.toFixed(1)}` : '-\u221E'}
        </span>
      </div>

      {/* M/S */}
      <div style={{ display: 'flex', gap: 2, padding: '2px 3px 4px', justifyContent: 'center' }}>
        <button
          onClick={() => onMute?.()}
          style={{
            ...sBtn,
            color: muted ? hw.red : hw.textFaint,
            background: muted ? hw.redDim : hw.bg,
          }}
        >M</button>
        <button
          onClick={onSolo}
          style={{
            ...sBtn,
            color: soloed ? hw.yellow : hw.textFaint,
            background: soloed ? hw.yellowDim : hw.bg,
          }}
        >S</button>
      </div>
    </div>
  )
}

const sBtn: React.CSSProperties = {
  width: 22, height: 14,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 8, fontWeight: 700,
  borderRadius: 3,
  padding: 0,
}
