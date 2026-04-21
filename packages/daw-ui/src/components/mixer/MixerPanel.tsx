import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'
import { useMeterStore, DEFAULT_TRACK_METER } from '../../stores/meterStore'
import { DetachButton } from '../FloatingWindow'
import { useEffect, useState, useCallback } from 'react'

export function MixerPanel() {
  const { tracks, setVolume, setPan, toggleMute, toggleSolo } = useTrackStore()
  const { master, tracks: trackMeters, startListening } = useMeterStore()
  useEffect(() => { startListening() }, [])

  const allTracks = tracks.filter(t => t.kind !== 'Master')
  const masterTrack = tracks.find(t => t.kind === 'Master')

  return (
    <div style={{ height: '100%', background: 'rgba(255,255,255,0.02)', backdropFilter: hw.blur.sm, display: 'flex', flexDirection: 'column' }}>
      <div style={{
        height: 22, background: 'rgba(255,255,255,0.01)', borderBottom: `1px solid ${hw.border}`,
        display: 'flex', alignItems: 'center', padding: '0 4px 0 8px', gap: 4,
      }}>
        <span style={{ flex: 1, fontSize: 10, fontWeight: 600, color: hw.textMuted }}>Mixer</span>
        <DetachButton panelId="mixer" />
      </div>

      <div style={{
        flex: 1, display: 'flex',
        overflowX: 'auto', overflowY: 'hidden',
        padding: 4, gap: 2,
      }}>
        {/* dB scale ruler (once per mixer, to the left of the first strip) */}
        {allTracks.length > 0 && <DbScale />}

        {allTracks.map((track, idx) => {
          const meter = trackMeters[track.id] ?? DEFAULT_TRACK_METER
          return (
            <Strip key={track.id}
              name={track.name} color={track.color} number={idx}
              volumeDb={track.volume_db} pan={track.pan}
              muted={track.muted} soloed={track.soloed}
              peakL={meter.peakL} peakR={meter.peakR} rmsDb={meter.rms}
              onVolume={db => setVolume(track.id, db)}
              onPan={p => setPan(track.id, p)}
              onMute={() => toggleMute(track.id)}
              onSolo={() => toggleSolo(track.id)}
            />
          )
        })}

        {allTracks.length > 0 && <div style={{ width: 1, background: hw.border, margin: '0 1px', flexShrink: 0 }} />}

        <Strip
          name="Master" color={hw.accent} number={-1}
          volumeDb={masterTrack?.volume_db ?? 0} pan={0}
          muted={masterTrack?.muted ?? false} soloed={false}
          peakL={master.peak_db} peakR={master.peak_db}
          rmsDb={master.rms_db}
          peakHoldDb={master.peak_hold_db}
          isMaster
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

// Meter dB range used by every strip.
const METER_MIN_DB = -60
const METER_MAX_DB = 6

function dbToPct(db: number) {
  const clamped = Math.max(METER_MIN_DB, Math.min(METER_MAX_DB, db))
  return ((clamped - METER_MIN_DB) / (METER_MAX_DB - METER_MIN_DB)) * 100
}

function DbScale() {
  const marks = [6, 0, -6, -12, -24, -36, -48, -60]
  return (
    <div data-testid="db-scale" style={{
      width: 18, flexShrink: 0, position: 'relative',
      display: 'flex', flexDirection: 'column',
      paddingTop: 22 /* match color bar + header area so ticks align with meter top */,
      paddingBottom: 30 /* dB readout + M/S row */,
    }}>
      {/* We align ticks to the visible meter height. The meter lives inside Strip
          starting after: color bar (2) + header (~22) + FX slots (~42) + clip led (6+gap).
          Easier: render ticks on a relative span that Strip layout also honors. */}
      <div style={{ flex: 1, position: 'relative' }}>
        {marks.map(db => {
          // Meters in Strip occupy the "fader row" area. We map db→pct of that area.
          const pct = 100 - dbToPct(db)
          return (
            <div key={db} data-testid={`db-mark-${db}`} style={{
              position: 'absolute', right: 1, left: 0, top: `${pct}%`,
              transform: 'translateY(-50%)',
              display: 'flex', alignItems: 'center', justifyContent: 'flex-end',
              gap: 2,
            }}>
              <span style={{ fontSize: 7, color: hw.textFaint, fontFamily: "'Consolas', monospace" }}>
                {db > 0 ? `+${db}` : db}
              </span>
              <div style={{ width: 3, height: 1, background: hw.textFaint, opacity: 0.5 }} />
            </div>
          )
        })}
      </div>
    </div>
  )
}

interface StripProps {
  name: string; color: string; number: number
  volumeDb: number; pan: number; muted: boolean; soloed: boolean
  peakL: number; peakR: number; rmsDb: number
  peakHoldDb?: number
  isMaster?: boolean
  onVolume?: (db: number) => void; onPan: (p: number) => void
  onMute?: () => void; onSolo: () => void
}

function Strip({ name, color, number, volumeDb, muted, soloed, peakL, peakR, rmsDb, peakHoldDb, isMaster, onVolume, onMute, onSolo }: StripProps) {
  const [clipped, setClipped] = useState(false)
  const resetClip = useCallback(() => setClipped(false), [])
  useEffect(() => { if (peakL >= 0 || peakR >= 0) setClipped(true) }, [peakL, peakR])

  const meterColor = (db: number) => db > -3 ? hw.red : db > -12 ? hw.yellow : hw.green

  return (
    <div data-testid={`mixer-strip-${isMaster ? 'master' : name}`} style={{
      width: 62, minWidth: 62,
      display: 'flex', flexDirection: 'column',
      background: 'rgba(255,255,255,0.03)', border: `1px solid ${hw.border}`,
      borderRadius: hw.radius.lg, flexShrink: 0, overflow: 'hidden',
    }}>
      {/* Color bar */}
      <div style={{ height: 2, background: color }} />

      {/* Header */}
      <div style={{ padding: '3px 3px 2px', textAlign: 'center', borderBottom: `1px solid ${hw.border}` }}>
        <div style={{ fontSize: 8, color: hw.textFaint }}>{isMaster ? 'M' : number}</div>
        <div style={{
          fontSize: 9, fontWeight: 500, color: hw.textPrimary,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>{name}</div>
      </div>

      {/* FX slots */}
      <div style={{ padding: '2px 3px', borderBottom: `1px solid ${hw.border}` }}>
        {[0, 1, 2].map(i => (
          <div key={i} style={{
            height: 11, background: 'rgba(255,255,255,0.03)', border: `1px solid ${hw.borderDark}`,
            borderRadius: hw.radius.sm, marginBottom: 1,
            display: 'flex', alignItems: 'center', padding: '0 3px',
          }}>
            <span style={{ fontSize: 7, color: hw.textFaint }}>{i + 1}</span>
          </div>
        ))}
      </div>

      {/* Fader + meter */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', padding: 3, gap: 2, minHeight: 70 }}>
        {/* Clip indicator */}
        <div
          onClick={resetClip}
          title={clipped ? 'Click to reset clip indicator' : undefined}
          style={{
            width: '100%', height: 6, borderRadius: hw.radius.sm,
            background: clipped ? hw.red : 'rgba(255,255,255,0.04)',
            border: `1px solid ${clipped ? hw.red : hw.borderDark}`,
            cursor: clipped ? 'pointer' : 'default',
            transition: 'background 0.1s',
            flexShrink: 0,
          }}
        />
        <div style={{ flex: 1, display: 'flex', gap: 2 }}>
        {/* Dual meter with RMS overlay */}
        <div style={{ display: 'flex', gap: 1, width: 10, flexShrink: 0 }}>
          {[
            { db: peakL, label: 'L' },
            { db: peakR, label: 'R' },
          ].map(({ db, label }) => (
            <div key={label} data-testid={`meter-${isMaster ? 'master' : name}-${label}`} style={{
              flex: 1, background: 'rgba(255,255,255,0.03)', position: 'relative',
              border: `1px solid ${hw.borderDark}`, borderRadius: hw.radius.sm,
              overflow: 'hidden',
            }}>
              {/* Peak bar */}
              <div data-testid={`meter-peak-${isMaster ? 'master' : name}-${label}`} data-db={db.toFixed(2)} style={{
                position: 'absolute', bottom: 0, left: 0, right: 0,
                height: `${dbToPct(db)}%`, background: meterColor(db),
                transition: 'height 60ms',
              }} />
              {/* RMS overlay (translucent lighter band) */}
              <div data-testid={`meter-rms-${isMaster ? 'master' : name}-${label}`} data-db={rmsDb.toFixed(2)} style={{
                position: 'absolute', bottom: 0, left: 0, right: 0,
                height: `${dbToPct(rmsDb)}%`,
                background: 'rgba(255,255,255,0.35)',
                mixBlendMode: 'overlay',
                transition: 'height 100ms',
              }} />
              {/* Peak-hold tick (master only — per-track meters don't track hold yet) */}
              {peakHoldDb !== undefined && peakHoldDb > METER_MIN_DB && (
                <div style={{
                  position: 'absolute', left: 0, right: 0,
                  bottom: `${dbToPct(peakHoldDb)}%`, height: 1,
                  background: hw.textPrimary,
                }} />
              )}
            </div>
          ))}
        </div>

        {/* Fader */}
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <input type="range" min={-60} max={12} step={0.1} value={volumeDb}
            onChange={e => onVolume?.(parseFloat(e.target.value))}
            style={{
              writingMode: 'vertical-lr' as any, direction: 'rtl',
              flex: 1, width: 16, appearance: 'none',
              background: 'rgba(255,255,255,0.04)',
              border: `1px solid ${hw.borderDark}`, borderRadius: hw.radius.sm, cursor: 'pointer',
            }}
          />
        </div>
        </div>
      </div>

      {/* dB readout */}
      <div style={{ textAlign: 'center', padding: '2px 0', borderTop: `1px solid ${hw.border}` }}>
        <span style={{ fontSize: 8, color: hw.textPrimary, fontFamily: "'Consolas', monospace" }}>
          {volumeDb > -60 ? `${volumeDb.toFixed(1)}` : '-\u221E'}
        </span>
      </div>

      {/* M/S buttons */}
      <div style={{ display: 'flex', gap: 1, padding: '2px 3px 3px', justifyContent: 'center' }}>
        <button onClick={() => onMute?.()} style={{
          ...sB, color: muted ? hw.red : hw.textMuted,
          background: muted ? hw.redDim : 'rgba(255,255,255,0.03)',
        }}>M</button>
        <button onClick={onSolo} style={{
          ...sB, color: soloed ? hw.yellow : hw.textMuted,
          background: soloed ? hw.yellowDim : 'rgba(255,255,255,0.03)',
        }}>S</button>
      </div>
    </div>
  )
}

const sB: React.CSSProperties = {
  width: 22, height: 14, display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 7, fontWeight: 700, borderRadius: 6, padding: 0,
  border: `1px solid rgba(255,255,255,0.06)`,
}
