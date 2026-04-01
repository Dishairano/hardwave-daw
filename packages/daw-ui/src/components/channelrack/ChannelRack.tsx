import { useState } from 'react'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'

const STEPS = 16

export function ChannelRack() {
  const { tracks, selectedTrackId, selectTrack, toggleMute } = useTrackStore()
  const channels = tracks.filter(t => t.kind !== 'Master')
  const [steps, setSteps] = useState<Record<string, boolean[]>>({})
  const [channelVolumes] = useState<Record<string, number>>({})
  const [channelPans] = useState<Record<string, number>>({})
  const [swing] = useState(0)
  const [graphEditor, setGraphEditor] = useState(false)

  const getSteps = (id: string): boolean[] => steps[id] || new Array(STEPS).fill(false)
  const getVol = (id: string) => channelVolumes[id] ?? 0.78
  const getPan = (id: string) => channelPans[id] ?? 0.5

  const toggleStep = (id: string, i: number) => {
    const cur = getSteps(id)
    const upd = [...cur]
    upd[i] = !upd[i]
    setSteps(prev => ({ ...prev, [id]: upd }))
  }

  return (
    <div style={{ height: '100%', background: '#3E3E3E', display: 'flex', flexDirection: 'column' }}>
      {/* Top toolbar */}
      <div style={{
        height: 26, background: '#333',
        borderBottom: '1px solid rgba(0,0,0,0.4)',
        display: 'flex', alignItems: 'center', padding: '0 4px', gap: 2,
      }}>
        <button style={topBtn} title="Channel options">
          <svg width="8" height="8" viewBox="0 0 8 8"><path d="M1 2h6M1 4h6M1 6h6" stroke="#999" strokeWidth="1"/></svg>
        </button>

        <div style={{ display: 'flex', gap: 1 }}>
          {['All', 'Audio', 'MIDI'].map(g => (
            <button key={g} style={{
              ...topBtn, width: 'auto', padding: '0 6px',
              fontSize: 9, color: g === 'All' ? '#FFF' : '#999',
              background: g === 'All' ? '#555' : 'transparent',
              border: `1px solid ${g === 'All' ? 'rgba(255,255,255,0.1)' : 'transparent'}`,
            }}>
              {g}
            </button>
          ))}
        </div>

        <TbSep />

        <div style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
          <span style={{ fontSize: 8, color: '#888' }}>SWG</span>
          <MiniKnob value={swing} color="#999" size={14} />
        </div>

        <TbSep />

        <div style={{ display: 'flex', alignItems: 'center', gap: 2 }}>
          <span style={{ fontSize: 8, color: '#888' }}>Steps</span>
          <span style={{
            fontSize: 10, color: hw.greenLcd, fontWeight: 700,
            fontFamily: "'Consolas', monospace",
            background: '#1A1A1A', padding: '1px 5px', borderRadius: 1,
            border: '1px solid rgba(0,0,0,0.5)',
          }}>
            {STEPS}
          </span>
        </div>

        <div style={{ flex: 1 }} />

        <button
          onClick={() => setGraphEditor(v => !v)}
          style={{
            ...topBtn, width: 'auto', padding: '0 6px', fontSize: 9,
            color: graphEditor ? '#FFF' : '#888',
            background: graphEditor ? '#555' : 'transparent',
          }}
        >
          Graph
        </button>

        <span style={{
          fontSize: 10, color: '#CCC', fontWeight: 600,
          background: '#333', padding: '2px 10px', borderRadius: 2,
          border: '1px solid rgba(0,0,0,0.4)',
        }}>
          Pattern 1
        </span>
      </div>

      {/* Channel rows */}
      <div style={{ flex: 1, overflowY: 'auto' }}>
        {channels.map((ch, ci) => {
          const selected = selectedTrackId === ch.id
          const vol = getVol(ch.id)
          const pan = getPan(ch.id)
          return (
            <div
              key={ch.id}
              style={{
                height: 30, display: 'flex', alignItems: 'stretch',
                borderBottom: '1px solid rgba(0,0,0,0.2)',
                background: selected ? 'rgba(85,136,187,0.12)' : (ci % 2 === 1 ? '#3B3B3B' : '#3E3E3E'),
              }}
            >
              {/* 1. Green LED */}
              <div
                onClick={() => toggleMute(ch.id)}
                style={{
                  width: 20, display: 'flex', alignItems: 'center', justifyContent: 'center',
                  cursor: 'pointer',
                }}
              >
                <div style={{
                  width: 8, height: 8, borderRadius: '50%',
                  background: ch.muted ? '#444' : '#00CC44',
                  boxShadow: ch.muted ? 'none' : '0 0 4px rgba(0, 204, 68, 0.6)',
                }} />
              </div>

              {/* 2. Pan knob */}
              <div style={{ width: 22, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <MiniKnob value={pan} color="#DDAA00" size={14} />
              </div>

              {/* 3. Volume knob */}
              <div style={{ width: 22, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <MiniKnob value={vol} color="#00CC44" size={14} />
              </div>

              {/* 4. Mixer track # */}
              <div style={{
                width: 20, display: 'flex', alignItems: 'center', justifyContent: 'center',
                fontSize: 9, color: '#888', fontFamily: "'Consolas', monospace",
                borderRight: '1px solid rgba(0,0,0,0.2)',
              }}>
                {ci + 1}
              </div>

              {/* 5. Channel name */}
              <div
                onClick={() => selectTrack(ch.id)}
                style={{
                  width: 110, minWidth: 110, display: 'flex', alignItems: 'center',
                  padding: '0 6px', cursor: 'default',
                  background: selected ? '#4A5A6A' : '#444',
                  borderRight: '1px solid rgba(0,0,0,0.2)',
                  gap: 4,
                }}
              >
                <div style={{
                  width: 3, height: 18, borderRadius: 1,
                  background: ch.color, flexShrink: 0,
                }} />
                <span style={{
                  fontSize: 10, color: selected ? '#FFF' : '#CCC',
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  fontWeight: selected ? 600 : 400,
                }}>
                  {ch.name}
                </span>
              </div>

              {/* 6. Channel select dot */}
              <div style={{
                width: 16, display: 'flex', alignItems: 'center', justifyContent: 'center',
                borderRight: '1px solid rgba(0,0,0,0.2)',
              }}>
                <div style={{
                  width: 6, height: 6, borderRadius: '50%',
                  background: selected ? '#00CC44' : 'transparent',
                  border: `1px solid ${selected ? '#00CC44' : '#555'}`,
                }} />
              </div>

              {/* 7. Step sequencer — ORANGE buttons */}
              <div style={{ flex: 1, display: 'flex', alignItems: 'center', padding: '0 4px', gap: 1, overflow: 'hidden' }}>
                {Array.from({ length: STEPS }, (_, i) => {
                  const active = getSteps(ch.id)[i]
                  const groupIdx = Math.floor(i / 4)
                  const isOddGroup = groupIdx % 2 === 1
                  return (
                    <button
                      key={i}
                      onClick={() => toggleStep(ch.id, i)}
                      style={{
                        flex: 1, maxWidth: 28, height: 22,
                        background: active
                          ? '#E85D00'  // FL Studio orange
                          : (isOddGroup ? '#4A4A4A' : '#444'),
                        border: `1px solid ${active ? '#FF7722' : 'rgba(0,0,0,0.3)'}`,
                        borderRadius: 2,
                        boxShadow: active ? '0 0 4px rgba(232, 93, 0, 0.3)' : 'none',
                        marginRight: i % 4 === 3 ? 4 : 0,
                        transition: 'background 0.05s',
                      }}
                    />
                  )
                })}
              </div>
            </div>
          )
        })}

        {channels.length === 0 && (
          <div style={{ padding: 20, textAlign: 'center', color: '#777', fontSize: 10 }}>
            No channels — add instruments to get started
          </div>
        )}
      </div>

      {/* Graph editor */}
      {graphEditor && (
        <div style={{
          height: 60, background: '#2A2A2A',
          borderTop: '1px solid rgba(0,0,0,0.4)',
          display: 'flex', alignItems: 'flex-end', padding: '4px 4px 2px',
          gap: 1, marginLeft: 210,
        }}>
          {Array.from({ length: STEPS }, (_, i) => (
            <div key={i} style={{
              flex: 1, maxWidth: 28,
              height: `${50 + Math.random() * 50}%`,
              background: 'rgba(232, 93, 0, 0.3)',
              border: '1px solid rgba(232, 93, 0, 0.2)',
              borderRadius: '2px 2px 0 0',
              marginRight: i % 4 === 3 ? 4 : 0,
            }} />
          ))}
        </div>
      )}

      {/* Bottom bar */}
      <div style={{
        height: 22, background: '#333',
        borderTop: '1px solid rgba(0,0,0,0.4)',
        display: 'flex', alignItems: 'center', padding: '0 6px', gap: 4,
      }}>
        <span style={{ fontSize: 8, color: '#888' }}>{channels.length} channels</span>
        <div style={{ flex: 1 }} />
        <button style={{
          ...topBtn, width: 'auto', padding: '0 6px', fontSize: 9,
          color: '#B0B0B0', gap: 2, display: 'flex', alignItems: 'center',
        }}>
          <svg width="8" height="8" viewBox="0 0 8 8">
            <line x1="4" y1="1" x2="4" y2="7" stroke="currentColor" strokeWidth="1.2"/>
            <line x1="1" y1="4" x2="7" y2="4" stroke="currentColor" strokeWidth="1.2"/>
          </svg>
          Add
        </button>
      </div>
    </div>
  )
}

function MiniKnob({ value, color, size }: { value: number; color: string; size: number }) {
  const angle = -135 + value * 270
  const r = size / 2 - 1
  const cx = size / 2
  const cy = size / 2
  const rad = (angle * Math.PI) / 180
  const endX = cx + (r - 2) * Math.sin(rad)
  const endY = cy - (r - 2) * Math.cos(rad)

  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
      <circle cx={cx} cy={cy} r={r} fill="#2A2A2A" stroke="rgba(0,0,0,0.4)" strokeWidth="0.5" />
      <line x1={cx} y1={cy} x2={endX} y2={endY} stroke={color} strokeWidth="1.2" strokeLinecap="round" />
      <circle cx={cx} cy={cy} r="1" fill={color} opacity="0.4" />
    </svg>
  )
}

function TbSep() {
  return <div style={{ width: 1, height: 14, background: 'rgba(0,0,0,0.3)', margin: '0 2px' }} />
}

const topBtn: React.CSSProperties = {
  width: 22, height: 18,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: 'transparent', border: '1px solid transparent',
  borderRadius: 2, cursor: 'pointer',
}
