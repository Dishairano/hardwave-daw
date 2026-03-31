import { useState } from 'react'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'

const STEPS = 16

export function ChannelRack() {
  const { tracks, selectedTrackId, selectTrack, toggleMute } = useTrackStore()
  const channels = tracks.filter(t => t.kind !== 'Master')
  const [steps, setSteps] = useState<Record<string, boolean[]>>({})
  const [channelVolumes, setChannelVolumes] = useState<Record<string, number>>({})
  const [channelPans, setChannelPans] = useState<Record<string, number>>({})
  const [swing, setSwing] = useState(0)
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
    <div style={{ height: '100%', background: hw.bgPanel, display: 'flex', flexDirection: 'column' }}>
      {/* ── Top toolbar ── */}
      <div style={{
        height: 26, background: hw.bgDeep, borderBottom: `1px solid ${hw.borderDark}`,
        display: 'flex', alignItems: 'center', padding: '0 4px', gap: 2,
      }}>
        {/* Options menu arrow */}
        <button style={topBtn} title="Channel options">
          <svg width="8" height="8" viewBox="0 0 8 8"><path d="M1 2h6M1 4h6M1 6h6" stroke={hw.textFaint} strokeWidth="1"/></svg>
        </button>

        {/* Group filter buttons */}
        <div style={{ display: 'flex', gap: 1 }}>
          {['All', 'Audio', 'MIDI'].map(g => (
            <button key={g} style={{
              ...topBtn, width: 'auto', padding: '0 5px',
              fontSize: 8, color: g === 'All' ? hw.purple : hw.textFaint,
              background: g === 'All' ? hw.purpleDim : 'transparent',
              border: `1px solid ${g === 'All' ? hw.purple + '30' : 'transparent'}`,
            }}>
              {g}
            </button>
          ))}
        </div>

        <Sep />

        {/* Swing knob */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
          <span style={{ fontSize: 8, color: hw.textFaint }}>SWG</span>
          <MiniKnob value={swing} color={hw.textMuted} size={14} />
        </div>

        <Sep />

        {/* Pattern length */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 2 }}>
          <span style={{ fontSize: 8, color: hw.textFaint }}>Steps</span>
          <span style={{
            fontSize: 9, color: hw.purple, fontWeight: 700,
            fontFamily: "'Consolas', monospace",
            background: '#111114', padding: '0 4px', borderRadius: 2,
            border: `1px solid ${hw.border}`,
          }}>
            {STEPS}
          </span>
        </div>

        <div style={{ flex: 1 }} />

        {/* Graph editor toggle */}
        <button
          onClick={() => setGraphEditor(v => !v)}
          style={{
            ...topBtn, width: 'auto', padding: '0 5px', fontSize: 8,
            color: graphEditor ? hw.purple : hw.textFaint,
            background: graphEditor ? hw.purpleDim : 'transparent',
          }}
        >
          Graph
        </button>

        {/* Pattern name */}
        <span style={{
          fontSize: 9, color: hw.textSecondary, fontWeight: 600,
          background: '#111114', padding: '1px 8px', borderRadius: 2,
          border: `1px solid ${hw.border}`,
        }}>
          Pattern 1
        </span>
      </div>

      {/* ── Channel rows ── */}
      <div style={{ flex: 1, overflowY: 'auto' }}>
        {channels.map((ch, ci) => {
          const selected = selectedTrackId === ch.id
          const vol = getVol(ch.id)
          const pan = getPan(ch.id)
          return (
            <div
              key={ch.id}
              style={{
                height: 32, display: 'flex', alignItems: 'stretch',
                borderBottom: `1px solid ${hw.border}`,
                background: selected ? 'rgba(155, 109, 255, 0.06)' : (ci % 2 === 1 ? 'rgba(0,0,0,0.08)' : 'transparent'),
              }}
            >
              {/* 1. Mute LED */}
              <div
                onClick={() => toggleMute(ch.id)}
                style={{
                  width: 18, display: 'flex', alignItems: 'center', justifyContent: 'center',
                  cursor: 'pointer',
                }}
                title={ch.muted ? 'Unmute' : 'Mute'}
              >
                <div style={{
                  width: 7, height: 7, borderRadius: '50%',
                  background: ch.muted ? '#444' : '#4ADE80',
                  boxShadow: ch.muted ? 'none' : '0 0 4px rgba(74, 222, 128, 0.5)',
                  transition: 'background 0.1s',
                }} />
              </div>

              {/* 2. Pan knob */}
              <div style={{
                width: 20, display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <MiniKnob value={pan} color={hw.yellow} size={13} />
              </div>

              {/* 3. Volume knob */}
              <div style={{
                width: 20, display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <MiniKnob value={vol} color={hw.green} size={13} />
              </div>

              {/* 4. Mixer track # */}
              <div style={{
                width: 18, display: 'flex', alignItems: 'center', justifyContent: 'center',
                fontSize: 8, color: hw.textFaint, fontFamily: "'Consolas', monospace",
                borderRight: `1px solid ${hw.border}`,
              }}>
                {ci + 1}
              </div>

              {/* 5. Channel name button */}
              <div
                onClick={() => selectTrack(ch.id)}
                style={{
                  width: 100, minWidth: 100, display: 'flex', alignItems: 'center',
                  padding: '0 6px', cursor: 'default',
                  background: selected ? 'rgba(155, 109, 255, 0.08)' : 'transparent',
                  borderRight: `1px solid ${hw.border}`,
                  gap: 4,
                }}
              >
                {/* Color strip */}
                <div style={{
                  width: 3, height: 18, borderRadius: 1,
                  background: ch.color,
                  flexShrink: 0,
                }} />
                <span style={{
                  fontSize: 10, color: selected ? hw.textPrimary : hw.textSecondary,
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  fontWeight: selected ? 600 : 400,
                }}>
                  {ch.name}
                </span>
              </div>

              {/* 6. Channel select indicator */}
              <div style={{
                width: 14, display: 'flex', alignItems: 'center', justifyContent: 'center',
                borderRight: `1px solid ${hw.border}`,
              }}>
                <div style={{
                  width: 5, height: 5, borderRadius: '50%',
                  background: selected ? hw.purple : 'transparent',
                  border: `1px solid ${selected ? hw.purple : hw.textFaint + '40'}`,
                }} />
              </div>

              {/* 7. Step sequencer grid */}
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
                          ? ch.color
                          : (isOddGroup ? '#1C1C22' : '#191920'),
                        border: `1px solid ${active ? ch.color + '90' : 'rgba(255,255,255,0.04)'}`,
                        borderRadius: 2,
                        boxShadow: active ? `0 0 5px ${ch.color}40` : 'none',
                        marginRight: i % 4 === 3 ? 4 : 0,
                        opacity: active ? 1 : 0.9,
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
          <div style={{ padding: 20, textAlign: 'center', color: hw.textFaint, fontSize: 10 }}>
            No channels — add instruments to get started
          </div>
        )}
      </div>

      {/* ── Graph editor (velocity bars) ── */}
      {graphEditor && (
        <div style={{
          height: 60, background: '#161619',
          borderTop: `1px solid ${hw.borderDark}`,
          display: 'flex', alignItems: 'flex-end', padding: '4px 4px 2px',
          gap: 1, marginLeft: 170, // align with step grid
        }}>
          {Array.from({ length: STEPS }, (_, i) => (
            <div key={i} style={{
              flex: 1, maxWidth: 28,
              height: `${50 + Math.random() * 50}%`,
              background: hw.purpleDim,
              border: `1px solid ${hw.purple}30`,
              borderRadius: '2px 2px 0 0',
              marginRight: i % 4 === 3 ? 4 : 0,
            }} />
          ))}
        </div>
      )}

      {/* ── Bottom bar ── */}
      <div style={{
        height: 22, background: hw.bgDeep, borderTop: `1px solid ${hw.borderDark}`,
        display: 'flex', alignItems: 'center', padding: '0 6px', gap: 4,
      }}>
        <span style={{ fontSize: 8, color: hw.textFaint }}>{channels.length} channels</span>
        <div style={{ flex: 1 }} />

        {/* Add channel button */}
        <button style={{
          ...topBtn, width: 'auto', padding: '0 6px', fontSize: 9,
          color: hw.textMuted, gap: 2, display: 'flex', alignItems: 'center',
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

/* ── Mini knob ── */
function MiniKnob({ value, color, size }: { value: number; color: string; size: number }) {
  const angle = -135 + value * 270 // -135 to +135 degrees
  const r = size / 2 - 1
  const cx = size / 2
  const cy = size / 2
  const rad = (angle * Math.PI) / 180
  const endX = cx + (r - 2) * Math.sin(rad)
  const endY = cy - (r - 2) * Math.cos(rad)

  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
      {/* Track */}
      <circle cx={cx} cy={cy} r={r} fill="#111114" stroke="rgba(255,255,255,0.06)" strokeWidth="0.5" />
      {/* Indicator line */}
      <line x1={cx} y1={cy} x2={endX} y2={endY} stroke={color} strokeWidth="1.2" strokeLinecap="round" />
      {/* Center dot */}
      <circle cx={cx} cy={cy} r="1" fill={color} opacity="0.4" />
    </svg>
  )
}

function Sep() {
  return <div style={{ width: 1, height: 14, background: hw.border, margin: '0 2px' }} />
}

const topBtn: React.CSSProperties = {
  width: 20, height: 18,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: 'transparent', border: '1px solid transparent',
  borderRadius: 2, cursor: 'pointer',
}
