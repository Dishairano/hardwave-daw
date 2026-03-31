import { useState } from 'react'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'

const STEPS = 16

export function ChannelRack() {
  const { tracks, selectedTrackId, selectTrack, toggleMute } = useTrackStore()
  const audioTracks = tracks.filter(t => t.kind !== 'Master')
  const [steps, setSteps] = useState<Record<string, boolean[]>>({})

  const getSteps = (trackId: string): boolean[] => steps[trackId] || new Array(STEPS).fill(false)

  const toggleStep = (trackId: string, stepIdx: number) => {
    const current = getSteps(trackId)
    const updated = [...current]
    updated[stepIdx] = !updated[stepIdx]
    setSteps(prev => ({ ...prev, [trackId]: updated }))
  }

  return (
    <div style={{ height: '100%', background: hw.bgPanel, display: 'flex', flexDirection: 'column' }}>
      <div style={{
        height: 22, background: hw.bgDeep, borderBottom: `1px solid ${hw.borderDark}`,
        display: 'flex', alignItems: 'center', padding: '0 8px',
      }}>
        <span style={{ fontSize: 10, fontWeight: 600, color: hw.textMuted }}>Channel Rack</span>
        <div style={{ flex: 1 }} />
        <span style={{ fontSize: 9, color: hw.textFaint }}>Pattern 1</span>
      </div>

      <div style={{ flex: 1, overflowY: 'auto' }}>
        {audioTracks.map(track => (
          <div
            key={track.id}
            style={{
              height: 28, display: 'flex', alignItems: 'stretch',
              borderBottom: `1px solid ${hw.border}`,
              background: selectedTrackId === track.id ? hw.bgHover : 'transparent',
            }}
          >
            <div
              onClick={() => selectTrack(track.id)}
              style={{
                width: 110, minWidth: 110,
                display: 'flex', alignItems: 'center', gap: 5,
                padding: '0 6px', cursor: 'default',
                borderRight: `1px solid ${hw.border}`,
              }}
            >
              <div
                style={{
                  width: 7, height: 7, borderRadius: '50%',
                  background: track.muted ? hw.textFaint : track.color,
                  boxShadow: track.muted ? 'none' : `0 0 5px ${track.color}50`,
                }}
                onClick={e => { e.stopPropagation(); toggleMute(track.id) }}
              />
              <span style={{
                fontSize: 10, color: hw.textSecondary,
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>
                {track.name}
              </span>
            </div>

            <div style={{ flex: 1, display: 'flex', alignItems: 'center', padding: '0 4px', gap: 1 }}>
              {Array.from({ length: STEPS }, (_, i) => {
                const active = getSteps(track.id)[i]
                const isOddGroup = Math.floor(i / 4) % 2 === 1
                return (
                  <button
                    key={i}
                    onClick={() => toggleStep(track.id, i)}
                    style={{
                      width: 20, height: 18,
                      background: active ? hw.purple : (isOddGroup ? '#1E1E24' : hw.bgInput),
                      border: `1px solid ${active ? hw.purple + '80' : hw.border}`,
                      borderRadius: 2,
                      boxShadow: active ? `0 0 6px ${hw.purpleGlow}` : 'none',
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
          <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 10 }}>
            Add tracks to populate channels
          </div>
        )}
      </div>
    </div>
  )
}
