import { useState } from 'react'
import { useTrackStore } from '../../stores/trackStore'

const STEPS = 16
const STEP_COLORS = ['#FF6B00', '#FF8533', '#FFA366', '#FFBB88']

export function ChannelRack() {
  const { tracks, selectedTrackId, selectTrack, toggleMute } = useTrackStore()
  const audioTracks = tracks.filter(t => t.kind !== 'Master')

  // Local step sequencer state (visual only for now)
  const [steps, setSteps] = useState<Record<string, boolean[]>>({})
  const [currentPattern, setCurrentPattern] = useState(1)

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
      background: '#262626',
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Header */}
      <div style={{
        height: 22,
        background: '#1E1E1E',
        borderBottom: '1px solid #333',
        display: 'flex',
        alignItems: 'center',
        padding: '0 8px',
        gap: 8,
      }}>
        <span style={{ fontSize: 9, fontWeight: 700, color: '#888', letterSpacing: 1 }}>
          CHANNEL RACK
        </span>
        <div style={{ flex: 1 }} />

        {/* Pattern selector */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 4,
          background: '#222', borderRadius: 3, border: '1px solid #444',
          padding: '1px 6px',
        }}>
          <button
            onClick={() => setCurrentPattern(Math.max(1, currentPattern - 1))}
            style={patNavBtn}
          >-</button>
          <span style={{
            fontSize: 10, fontWeight: 700, color: '#FF6B00',
            fontFamily: "'Courier New', monospace", minWidth: 20, textAlign: 'center',
          }}>
            {currentPattern}
          </span>
          <button
            onClick={() => setCurrentPattern(currentPattern + 1)}
            style={patNavBtn}
          >+</button>
        </div>
        <span style={{ fontSize: 8, color: '#555' }}>Pattern</span>
      </div>

      {/* Channel list + step grid */}
      <div style={{ flex: 1, overflowY: 'auto', overflowX: 'hidden' }}>
        {audioTracks.map((track) => (
          <div
            key={track.id}
            style={{
              height: 28,
              display: 'flex',
              alignItems: 'stretch',
              borderBottom: '1px solid #1E1E1E',
              background: selectedTrackId === track.id ? '#333' : '#282828',
            }}
          >
            {/* Channel button (left side) */}
            <div
              onClick={() => selectTrack(track.id)}
              style={{
                width: 120,
                minWidth: 120,
                display: 'flex',
                alignItems: 'center',
                gap: 4,
                padding: '0 4px',
                cursor: 'pointer',
                borderRight: '1px solid #1E1E1E',
              }}
            >
              {/* LED indicator */}
              <div style={{
                width: 8, height: 8, borderRadius: '50%',
                background: track.muted ? '#444' : track.color,
                border: '1px solid rgba(0,0,0,0.3)',
                cursor: 'pointer',
                flexShrink: 0,
              }}
                onClick={(e) => { e.stopPropagation(); toggleMute(track.id) }}
              />

              {/* Channel name */}
              <span style={{
                fontSize: 10, color: '#CCC', fontWeight: 500,
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>
                {track.name}
              </span>
            </div>

            {/* Step buttons */}
            <div style={{
              flex: 1, display: 'flex', alignItems: 'center',
              padding: '0 4px', gap: 1,
            }}>
              {Array.from({ length: STEPS }, (_, i) => {
                const active = getSteps(track.id)[i]
                const groupIdx = Math.floor(i / 4)
                const stepColor = STEP_COLORS[groupIdx % STEP_COLORS.length]
                return (
                  <button
                    key={i}
                    onClick={() => toggleStep(track.id, i)}
                    style={{
                      width: 22,
                      height: 20,
                      border: 'none',
                      borderRadius: 2,
                      background: active ? stepColor : '#1E1E1E',
                      opacity: active ? 1 : 0.6,
                      cursor: 'pointer',
                      flexShrink: 0,
                      boxShadow: active ? `0 0 4px ${stepColor}44` : 'none',
                      marginRight: i % 4 === 3 ? 4 : 0,
                    }}
                  />
                )
              })}
            </div>
          </div>
        ))}

        {audioTracks.length === 0 && (
          <div style={{ padding: 16, textAlign: 'center', color: '#444', fontSize: 10 }}>
            Add channels from<br />the toolbar
          </div>
        )}
      </div>
    </div>
  )
}

const patNavBtn: React.CSSProperties = {
  width: 14, height: 14,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 10, fontWeight: 700, color: '#888',
  background: 'transparent', border: 'none',
  cursor: 'pointer', padding: 0,
}
