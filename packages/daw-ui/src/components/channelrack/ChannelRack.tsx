import { useState } from 'react'
import { useTrackStore } from '../../stores/trackStore'

const STEPS = 16

// FL Studio step colors — each beat group (4 steps) has a distinct shade
const STEP_GROUP_COLORS = [
  { active: '#E8A030', inactive: '#3A3020' },
  { active: '#D09028', inactive: '#332818' },
  { active: '#E8A030', inactive: '#3A3020' },
  { active: '#D09028', inactive: '#332818' },
]

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
      background: '#1D1D1D',
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Header — FL style with pattern info */}
      <div style={{
        height: 20,
        background: '#252525',
        borderBottom: '1px solid #111',
        display: 'flex',
        alignItems: 'center',
        padding: '0 6px',
        gap: 6,
      }}>
        <span style={{ fontSize: 10, color: '#777' }}>Channel rack</span>
        <div style={{ flex: 1 }} />
        <span style={{ fontSize: 9, color: '#555' }}>Pattern 1</span>
      </div>

      {/* Channel list + step grid */}
      <div style={{ flex: 1, overflowY: 'auto', overflowX: 'hidden' }}>
        {audioTracks.map((track) => (
          <div
            key={track.id}
            style={{
              height: 26,
              display: 'flex',
              alignItems: 'stretch',
              borderBottom: '1px solid #151515',
              background: selectedTrackId === track.id ? '#2A2A2A' : '#1D1D1D',
            }}
          >
            {/* Channel button — FL has round LED + name */}
            <div
              onClick={() => selectTrack(track.id)}
              style={{
                width: 110,
                minWidth: 110,
                display: 'flex',
                alignItems: 'center',
                gap: 4,
                padding: '0 4px',
                cursor: 'default',
                borderRight: '1px solid #111',
              }}
            >
              {/* Round LED (FL-style green circle, dim when muted) */}
              <div
                style={{
                  width: 7, height: 7, borderRadius: '50%',
                  background: track.muted ? '#333' : track.color,
                  boxShadow: track.muted ? 'none' : `0 0 3px ${track.color}44`,
                  flexShrink: 0,
                  cursor: 'default',
                }}
                onClick={(e) => { e.stopPropagation(); toggleMute(track.id) }}
              />
              <span style={{
                fontSize: 10, color: '#B0B0B0',
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>
                {track.name}
              </span>
            </div>

            {/* Step buttons — FL colored squares with beat grouping */}
            <div style={{
              flex: 1, display: 'flex', alignItems: 'center',
              padding: '0 3px', gap: 1,
            }}>
              {Array.from({ length: STEPS }, (_, i) => {
                const active = getSteps(track.id)[i]
                const group = STEP_GROUP_COLORS[Math.floor(i / 4) % STEP_GROUP_COLORS.length]
                return (
                  <button
                    key={i}
                    onClick={() => toggleStep(track.id, i)}
                    style={{
                      width: 20, height: 18,
                      border: 'none',
                      background: active ? group.active : group.inactive,
                      cursor: 'default',
                      flexShrink: 0,
                      marginRight: i % 4 === 3 ? 3 : 0,
                    }}
                  />
                )
              })}
            </div>
          </div>
        ))}

        {audioTracks.length === 0 && (
          <div style={{ padding: 12, textAlign: 'center', color: '#444', fontSize: 10 }}>
            Add channels from the toolbar
          </div>
        )}
      </div>
    </div>
  )
}
