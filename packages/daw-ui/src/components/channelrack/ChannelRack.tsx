import { useState } from 'react'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'

const STEPS = 16

export function ChannelRack() {
  const { tracks, selectedTrackId, selectTrack, toggleMute } = useTrackStore()
  const audioTracks = tracks.filter(t => t.kind !== 'Master')

  const [steps, setSteps] = useState<Record<string, boolean[]>>({})

  const getSteps = (trackId: string): boolean[] => {
    return steps[trackId] || new Array(STEPS).fill(false)
  }

  const toggleStep = (trackId: string, stepIdx: number) => {
    const current = getSteps(trackId)
    const updated = [...current]
    updated[stepIdx] = !updated[stepIdx]
    setSteps(prev => ({ ...prev, [trackId]: updated }))
  }

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
        gap: 8,
      }}>
        <span style={{ fontSize: 11, fontWeight: 600, color: hw.textSecondary }}>Channel Rack</span>
        <div style={{ flex: 1 }} />
        <span style={{ fontSize: 10, color: hw.textFaint }}>Pattern 1</span>
      </div>

      {/* Channel rows */}
      <div style={{ flex: 1, overflowY: 'auto' }}>
        {audioTracks.map((track) => (
          <div
            key={track.id}
            style={{
              height: 30,
              display: 'flex',
              alignItems: 'stretch',
              borderBottom: `1px solid ${hw.border}`,
              background: selectedTrackId === track.id ? hw.bgHover : 'transparent',
            }}
          >
            {/* Channel name */}
            <div
              onClick={() => selectTrack(track.id)}
              style={{
                width: 120,
                minWidth: 120,
                display: 'flex',
                alignItems: 'center',
                gap: 6,
                padding: '0 8px',
                cursor: 'default',
                borderRight: `1px solid ${hw.border}`,
              }}
            >
              <div
                style={{
                  width: 8, height: 8, borderRadius: '50%',
                  background: track.muted ? hw.textFaint : track.color,
                  boxShadow: track.muted ? 'none' : `0 0 6px ${track.color}40`,
                  cursor: 'pointer',
                }}
                onClick={(e) => { e.stopPropagation(); toggleMute(track.id) }}
              />
              <span style={{
                fontSize: 11, color: hw.textSecondary,
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>
                {track.name}
              </span>
            </div>

            {/* Step buttons */}
            <div style={{
              flex: 1, display: 'flex', alignItems: 'center',
              padding: '0 6px', gap: 2,
            }}>
              {Array.from({ length: STEPS }, (_, i) => {
                const active = getSteps(track.id)[i]
                return (
                  <button
                    key={i}
                    onClick={() => toggleStep(track.id, i)}
                    style={{
                      width: 20, height: 20,
                      borderRadius: hw.radius.sm,
                      background: active ? hw.red : hw.bgCard,
                      border: `1px solid ${active ? hw.red : hw.border}`,
                      boxShadow: active ? `0 0 6px ${hw.redGlow}` : 'none',
                      flexShrink: 0,
                      marginRight: i % 4 === 3 ? 4 : 0,
                    }}
                  />
                )
              })}
            </div>
          </div>
        ))}

        {audioTracks.length === 0 && (
          <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 11 }}>
            Add tracks to populate channels
          </div>
        )}
      </div>
    </div>
  )
}
