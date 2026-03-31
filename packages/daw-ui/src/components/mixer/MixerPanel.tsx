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
    <div style={{ height: '100%', background: hw.bgDark, display: 'flex', flexDirection: 'column' }}>
      <div style={{
        height: 22, background: hw.bgDeep, borderBottom: `1px solid ${hw.borderDark}`,
        display: 'flex', alignItems: 'center', padding: '0 8px',
      }}>
        <span style={{ fontSize: 10, fontWeight: 600, color: hw.textMuted }}>Mixer</span>
      </div>

      <div style={{
        flex: 1, display: 'flex',
        overflowX: 'auto', overflowY: 'hidden',
        padding: 4, gap: 2,
      }}>
        {allTracks.map((track, idx) => (
          <Strip key={track.id}
            name={track.name} color={track.color} number={idx}
            volumeDb={track.volume_db} pan={track.pan}
            muted={track.muted} soloed={track.soloed} peakDb={-60}
            onVolume={db => setVolume(track.id, db)}
            onPan={p => setPan(track.id, p)}
            onMute={() => toggleMute(track.id)}
            onSolo={() => toggleSolo(track.id)}
          />
        ))}

        {allTracks.length > 0 && <div style={{ width: 1, background: hw.border, margin: '0 1px', flexShrink: 0 }} />}

        <Strip
          name="Master" color={hw.purple} number={-1}
          volumeDb={masterTrack?.volume_db ?? 0} pan={0}
          muted={masterTrack?.muted ?? false} soloed={false}
          peakDb={master.peak_db} isMaster
          onVolume={masterTrack ? db => setVolume(masterTrack.id, db) : undefined}
          onPan={() => {}} onMute={masterTrack ? () => toggleMute(masterTrack.id) : undefined}
          onSolo={() => {}}
        />

        {allTracks.length === 0 && (
          <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', color: hw.textFaint, fontSize: 10 }}>
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
  onVolume?: (db: number) => void; onPan: (p: number) => void
  onMute?: () => void; onSolo: () => void
}

function Strip({ name, color, number, volumeDb, muted, soloed, peakDb, isMaster, onVolume, onMute, onSolo }: StripProps) {
  const mH = Math.max(0, Math.min(100, (peakDb + 60) / 72 * 100))
  const mC = peakDb > -3 ? '#EF4444' : peakDb > -12 ? hw.yellow : hw.green

  return (
    <div style={{
      width: 62, minWidth: 62,
      display: 'flex', flexDirection: 'column',
      background: hw.bgPanel, border: `1px solid ${hw.border}`,
      borderRadius: hw.radius.md, flexShrink: 0, overflow: 'hidden',
    }}>
      {/* Color bar */}
      <div style={{ height: 2, background: color }} />

      {/* Header */}
      <div style={{ padding: '3px 3px 2px', textAlign: 'center', borderBottom: `1px solid ${hw.border}` }}>
        <div style={{ fontSize: 8, color: hw.textFaint }}>{isMaster ? 'M' : number}</div>
        <div style={{
          fontSize: 9, fontWeight: 500, color: hw.textSecondary,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>{name}</div>
      </div>

      {/* FX slots */}
      <div style={{ padding: '2px 3px', borderBottom: `1px solid ${hw.border}` }}>
        {[0, 1, 2].map(i => (
          <div key={i} style={{
            height: 10, background: hw.bgInput, border: `1px solid ${hw.border}`,
            borderRadius: 1, marginBottom: 1,
            display: 'flex', alignItems: 'center', padding: '0 3px',
          }}>
            <span style={{ fontSize: 6, color: hw.textFaint }}>{i + 1}</span>
          </div>
        ))}
      </div>

      {/* Fader + meter */}
      <div style={{ flex: 1, display: 'flex', padding: 3, gap: 2, minHeight: 70 }}>
        {/* Dual meter */}
        <div style={{ display: 'flex', gap: 0, width: 8, flexShrink: 0 }}>
          {[0, 1].map(ch => (
            <div key={ch} style={{
              flex: 1, background: hw.bgDeep, position: 'relative',
              border: `1px solid ${hw.border}`, borderRadius: 1,
            }}>
              <div style={{
                position: 'absolute', bottom: 0, left: 0, right: 0,
                height: `${mH}%`, background: mC,
                borderRadius: 1, transition: 'height 60ms',
                boxShadow: mH > 60 ? `0 0 4px ${mC}30` : 'none',
              }} />
            </div>
          ))}
        </div>

        {/* Fader */}
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <input type="range" min={-60} max={12} step={0.1} value={volumeDb}
            onChange={e => onVolume?.(parseFloat(e.target.value))}
            style={{
              writingMode: 'vertical-lr' as any, direction: 'rtl',
              flex: 1, width: 14, appearance: 'none',
              background: `linear-gradient(to bottom, ${hw.bgCard}, ${hw.bgDeep})`,
              border: `1px solid ${hw.border}`, borderRadius: 2, cursor: 'pointer',
            }}
          />
        </div>
      </div>

      {/* dB */}
      <div style={{ textAlign: 'center', padding: '1px 0', borderTop: `1px solid ${hw.border}` }}>
        <span style={{ fontSize: 8, color: hw.textFaint, fontFamily: "'Consolas', monospace" }}>
          {volumeDb > -60 ? `${volumeDb.toFixed(1)}` : '-\u221E'}
        </span>
      </div>

      {/* M/S */}
      <div style={{ display: 'flex', gap: 1, padding: '2px 3px 3px', justifyContent: 'center' }}>
        <button onClick={() => onMute?.()} style={{
          ...sB, color: muted ? '#EF4444' : hw.textFaint,
          background: muted ? 'rgba(239,68,68,0.12)' : hw.bgInput,
        }}>M</button>
        <button onClick={onSolo} style={{
          ...sB, color: soloed ? hw.yellow : hw.textFaint,
          background: soloed ? 'rgba(251,191,36,0.12)' : hw.bgInput,
        }}>S</button>
      </div>
    </div>
  )
}

const sB: React.CSSProperties = {
  width: 22, height: 13, display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 7, fontWeight: 700, borderRadius: 2, padding: 0,
}
