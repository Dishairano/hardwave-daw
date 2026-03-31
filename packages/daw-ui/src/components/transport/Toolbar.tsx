import { useState } from 'react'
import { hw } from '../../theme'
import { useTransportStore } from '../../stores/transportStore'

type Tool = 'draw' | 'paint' | 'delete' | 'mute' | 'slip' | 'slice' | 'select' | 'zoom'

interface ToolbarProps {
  showBrowser: boolean
  showPlaylist: boolean
  showChannelRack: boolean
  showPianoRoll: boolean
  showMixer: boolean
  onToggleBrowser: () => void
  onTogglePlaylist: () => void
  onToggleChannelRack: () => void
  onTogglePianoRoll: () => void
  onToggleMixer: () => void
  onSetHint: (text: string) => void
}

export function Toolbar(props: ToolbarProps) {
  const { playing, bpm, positionSamples, sampleRate, togglePlayback, stop, setBpm } = useTransportStore()
  const [activeTool, setActiveTool] = useState<Tool>('draw')

  const seconds = sampleRate > 0 ? positionSamples / sampleRate : 0
  const mins = Math.floor(seconds / 60)
  const secs = Math.floor(seconds % 60)
  const cs = Math.floor((seconds % 1) * 100)
  const beats = bpm > 0 ? (seconds * bpm / 60) : 0
  const bar = Math.floor(beats / 4) + 1
  const beat = Math.floor(beats % 4) + 1
  const tick = Math.floor((beats % 1) * 960)

  const hint = (text: string) => () => props.onSetHint(text)
  const clear = () => props.onSetHint('')

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      height: 34,
      background: hw.bgToolbarGrad,
      borderBottom: `1px solid ${hw.borderDark}`,
      padding: '0 4px',
      gap: 1,
    }}>
      {/* 1. Panel shortcut buttons — FL order */}
      <div style={{ display: 'flex', gap: 0 }}>
        <IconBtn icon="playlist" active={props.showPlaylist} onClick={props.onTogglePlaylist} onEnter={hint('Playlist (F5)')} onLeave={clear} />
        <IconBtn icon="channel" active={props.showChannelRack} onClick={props.onToggleChannelRack} onEnter={hint('Channel Rack (F6)')} onLeave={clear} />
        <IconBtn icon="pianoroll" active={props.showPianoRoll} onClick={props.onTogglePianoRoll} onEnter={hint('Piano Roll (F7)')} onLeave={clear} />
        <IconBtn icon="mixer" active={props.showMixer} onClick={props.onToggleMixer} onEnter={hint('Mixer (F9)')} onLeave={clear} />
        <IconBtn icon="browser" active={props.showBrowser} onClick={props.onToggleBrowser} onEnter={hint('Browser')} onLeave={clear} />
      </div>

      <Sep />

      {/* 2. Pattern selector with +/- */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 0 }}>
        <button style={patNavBtn} onMouseEnter={hint('Previous pattern')} onMouseLeave={clear}>
          <svg width="5" height="7" viewBox="0 0 5 7"><path d="M4 0.5L1 3.5L4 6.5" stroke={hw.textFaint} strokeWidth="1" fill="none"/></svg>
        </button>
        <div style={{
          ...lcdBox, padding: '0 6px', cursor: 'default', minWidth: 70,
        }} onMouseEnter={hint('Select pattern')} onMouseLeave={clear}>
          <span style={{ fontSize: 10, color: hw.textSecondary }}>Pattern 1</span>
        </div>
        <button style={patNavBtn} onMouseEnter={hint('Next pattern')} onMouseLeave={clear}>
          <svg width="5" height="7" viewBox="0 0 5 7"><path d="M1 0.5L4 3.5L1 6.5" stroke={hw.textFaint} strokeWidth="1" fill="none"/></svg>
        </button>
      </div>

      <Sep />

      {/* 3. PAT / SONG */}
      <div style={{
        display: 'flex', background: hw.bgInput, border: `1px solid ${hw.border}`,
        overflow: 'hidden', borderRadius: 2,
      }}>
        <ModeBtn label="PAT" active onEnter={hint('Pattern mode')} onLeave={clear} />
        <div style={{ width: 1, background: hw.border }} />
        <ModeBtn label="SONG" active={false} onEnter={hint('Song mode')} onLeave={clear} />
      </div>

      <Sep />

      {/* 4. Transport — Record, Stop, Play (FL order) */}
      <div style={{ display: 'flex', gap: 0 }}>
        <button style={tBtn} onMouseEnter={hint('Record (R)')} onMouseLeave={clear}>
          <svg width="8" height="8"><circle cx="4" cy="4" r="3.5" fill="#8B3030" /></svg>
        </button>
        <button onClick={stop} style={tBtn} onMouseEnter={hint('Stop')} onMouseLeave={clear}>
          <svg width="8" height="8"><rect width="8" height="8" rx="1" fill={hw.textMuted} /></svg>
        </button>
        <button onClick={togglePlayback} style={{ ...tBtn, background: playing ? hw.greenDim : tBtn.background }} onMouseEnter={hint('Play (Space)')} onMouseLeave={clear}>
          <svg width="8" height="10"><polygon points="0,0 8,5 0,10" fill={playing ? hw.green : hw.textMuted} /></svg>
        </button>
      </div>

      <Sep />

      {/* 5. Tempo */}
      <div style={{ ...lcdBox, width: 62 }} onMouseEnter={hint('Tempo')} onMouseLeave={clear}>
        <input
          type="number" value={bpm}
          onChange={e => setBpm(parseFloat(e.target.value) || 140)}
          style={{
            width: 44, background: 'transparent', border: 'none',
            color: hw.purple, fontSize: 13, fontWeight: 700,
            fontFamily: "'Consolas', 'Courier New', monospace",
            textAlign: 'right', outline: 'none',
          }}
        />
      </div>

      <Sep />

      {/* 6. Time display — Bar:Beat:Tick | Min:Sec.Cs */}
      <div style={{ ...lcdBox, padding: '0 6px', gap: 6 }}>
        <span style={lcdText}>
          <span style={{ color: hw.purple }}>{String(bar).padStart(3, ' ')}</span>
          <span style={{ color: hw.textFaint }}>:</span>
          <span style={{ color: hw.purple }}>{beat}</span>
          <span style={{ color: hw.textFaint }}>:</span>
          <span style={{ color: hw.purple }}>{String(tick).padStart(3, '0')}</span>
        </span>
        <div style={{ width: 1, height: 12, background: hw.border }} />
        <span style={lcdText}>
          <span style={{ color: hw.green }}>{String(mins).padStart(2, ' ')}</span>
          <span style={{ color: hw.textFaint }}>:</span>
          <span style={{ color: hw.green }}>{String(secs).padStart(2, '0')}</span>
          <span style={{ color: hw.textFaint }}>.</span>
          <span style={{ color: hw.green }}>{String(cs).padStart(2, '0')}</span>
        </span>
      </div>

      <Sep />

      {/* 7. Song position slider */}
      <div style={{
        width: 80, height: 6, background: hw.bgInput, borderRadius: 3,
        border: `1px solid ${hw.border}`, position: 'relative', cursor: 'pointer',
      }} onMouseEnter={hint('Song position')} onMouseLeave={clear}>
        <div style={{
          position: 'absolute', left: 0, top: 0, width: '0%', height: '100%',
          background: `linear-gradient(90deg, ${hw.purpleMuted}, ${hw.purple})`,
          borderRadius: 3,
        }} />
      </div>

      <Sep />

      {/* 8. Global snap */}
      <div style={{ ...lcdBox, padding: '0 6px', cursor: 'default' }} onMouseEnter={hint('Global snap')} onMouseLeave={clear}>
        <svg width="8" height="8" viewBox="0 0 8 8" style={{ marginRight: 3 }}>
          <path d="M1 7V1h6" stroke={hw.textFaint} strokeWidth="0.8" fill="none"/>
          <circle cx="4" cy="4" r="1.5" fill={hw.textMuted}/>
        </svg>
        <span style={{ fontSize: 10, color: hw.textMuted }}>Line</span>
        <svg width="7" height="5" viewBox="0 0 7 5" style={{ marginLeft: 3 }}>
          <path d="M0.5 0.5L3.5 4L6.5 0.5" stroke={hw.textFaint} strokeWidth="1" fill="none" />
        </svg>
      </div>

      <Sep />

      {/* 9. Tool buttons — FL Studio signature: Draw, Paint, Delete, Mute, Slip, Slice, Select, Zoom */}
      <div style={{ display: 'flex', gap: 0 }}>
        <ToolSelectBtn tool="draw" active={activeTool} onClick={setActiveTool} onEnter={hint('Draw (P)')} onLeave={clear}>
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M1.5 8.5L2 6L7 1L9 3L4 8Z" stroke="currentColor" strokeWidth="0.8" fill={activeTool === 'draw' ? 'currentColor' : 'none'} opacity={activeTool === 'draw' ? 0.3 : 1}/><path d="M7 1L9 3" stroke="currentColor" strokeWidth="1"/></svg>
        </ToolSelectBtn>
        <ToolSelectBtn tool="paint" active={activeTool} onClick={setActiveTool} onEnter={hint('Paint (B)')} onLeave={clear}>
          <svg width="10" height="10" viewBox="0 0 10 10"><rect x="1" y="6" width="3" height="3.5" rx="0.5" stroke="currentColor" strokeWidth="0.8" fill="none"/><path d="M2.5 6V2.5C2.5 1.5 3.5 0.5 5 0.5H8C8.5 0.5 9 1 9 1.5V3C9 3.5 8.5 4 8 4H5.5L4 5.5" stroke="currentColor" strokeWidth="0.8" fill="none"/></svg>
        </ToolSelectBtn>
        <ToolSelectBtn tool="delete" active={activeTool} onClick={setActiveTool} onEnter={hint('Delete (D)')} onLeave={clear}>
          <svg width="10" height="10" viewBox="0 0 10 10"><line x1="2" y1="2" x2="8" y2="8" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round"/><line x1="8" y1="2" x2="2" y2="8" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round"/></svg>
        </ToolSelectBtn>
        <ToolSelectBtn tool="mute" active={activeTool} onClick={setActiveTool} onEnter={hint('Mute (T)')} onLeave={clear}>
          <svg width="10" height="10" viewBox="0 0 10 10"><rect x="1" y="1" width="8" height="8" rx="1" stroke="currentColor" strokeWidth="0.8" fill="none"/><line x1="1" y1="1" x2="9" y2="9" stroke="currentColor" strokeWidth="0.8"/></svg>
        </ToolSelectBtn>
        <ToolSelectBtn tool="slip" active={activeTool} onClick={setActiveTool} onEnter={hint('Slip (S)')} onLeave={clear}>
          <svg width="10" height="10" viewBox="0 0 10 10"><rect x="1" y="3" width="8" height="4" rx="0.5" stroke="currentColor" strokeWidth="0.8" fill="none"/><path d="M4 3V7M6 3V7" stroke="currentColor" strokeWidth="0.6" strokeDasharray="1 1"/></svg>
        </ToolSelectBtn>
        <ToolSelectBtn tool="slice" active={activeTool} onClick={setActiveTool} onEnter={hint('Slice (C)')} onLeave={clear}>
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M3 1L7 9" stroke="currentColor" strokeWidth="1" strokeLinecap="round"/><circle cx="3" cy="1.5" r="1" stroke="currentColor" strokeWidth="0.6" fill="none"/></svg>
        </ToolSelectBtn>
        <ToolSelectBtn tool="select" active={activeTool} onClick={setActiveTool} onEnter={hint('Select (E)')} onLeave={clear}>
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M2 1L2 9L5 6.5L7.5 9L8.5 8L6 5.5L9 5L2 1Z" stroke="currentColor" strokeWidth="0.7" fill={activeTool === 'select' ? 'currentColor' : 'none'} opacity={activeTool === 'select' ? 0.3 : 1}/></svg>
        </ToolSelectBtn>
        <ToolSelectBtn tool="zoom" active={activeTool} onClick={setActiveTool} onEnter={hint('Zoom (Z)')} onLeave={clear}>
          <svg width="10" height="10" viewBox="0 0 10 10"><circle cx="4.5" cy="4.5" r="3" stroke="currentColor" strokeWidth="0.9" fill="none"/><line x1="7" y1="7" x2="9.5" y2="9.5" stroke="currentColor" strokeWidth="1" strokeLinecap="round"/><line x1="3" y1="4.5" x2="6" y2="4.5" stroke="currentColor" strokeWidth="0.7"/><line x1="4.5" y1="3" x2="4.5" y2="6" stroke="currentColor" strokeWidth="0.7"/></svg>
        </ToolSelectBtn>
      </div>

      <Sep />

      {/* 10. Multilink to controllers */}
      <ToolBtn onEnter={hint('Multilink to controllers')} onLeave={clear}>
        <svg width="11" height="11" viewBox="0 0 11 11">
          <circle cx="3" cy="3" r="1.5" stroke="currentColor" strokeWidth="0.8" fill="none"/>
          <circle cx="8" cy="8" r="1.5" stroke="currentColor" strokeWidth="0.8" fill="none"/>
          <line x1="4.2" y1="4.2" x2="6.8" y2="6.8" stroke="currentColor" strokeWidth="0.8"/>
        </svg>
      </ToolBtn>

      <div style={{ flex: 1 }} />

      {/* 11. Typing keyboard to piano */}
      <ToolBtn onEnter={hint('Typing keyboard to piano (Ctrl+T)')} onLeave={clear}>
        <svg width="13" height="9" viewBox="0 0 13 9">
          <rect x="0.5" y="0.5" width="12" height="8" rx="1" fill="none" stroke="currentColor" strokeWidth="0.8"/>
          <rect x="2" y="2" width="2" height="2" rx="0.3" fill="currentColor" opacity="0.5"/>
          <rect x="5.5" y="2" width="2" height="2" rx="0.3" fill="currentColor" opacity="0.5"/>
          <rect x="9" y="2" width="2" height="2" rx="0.3" fill="currentColor" opacity="0.5"/>
          <rect x="3" y="5.5" width="7" height="1.5" rx="0.3" fill="currentColor" opacity="0.4"/>
        </svg>
      </ToolBtn>

      <Sep />

      {/* 12. Master volume */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 3 }} onMouseEnter={hint('Master volume')} onMouseLeave={clear}>
        <svg width="9" height="9" viewBox="0 0 9 9">
          <polygon points="0,4 3,1 3,7" fill={hw.textFaint} />
          <path d="M4.5 2.5 Q6 4.5 4.5 6.5" stroke={hw.textFaint} strokeWidth="0.8" fill="none"/>
          <path d="M5.5 1.5 Q8 4.5 5.5 7.5" stroke={hw.textFaint} strokeWidth="0.8" fill="none"/>
        </svg>
        <div style={{
          width: 50, height: 4, background: hw.bgInput, borderRadius: 2,
          border: `1px solid ${hw.border}`, position: 'relative',
        }}>
          <div style={{
            width: '80%', height: '100%', borderRadius: 2,
            background: `linear-gradient(90deg, ${hw.purpleMuted}, ${hw.purple})`,
          }} />
        </div>
      </div>

      <Sep />

      {/* 13. Master pitch knob */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 2 }} onMouseEnter={hint('Master pitch')} onMouseLeave={clear}>
        <span style={{ fontSize: 7, color: hw.textFaint }}>PIT</span>
        <div style={{
          width: 14, height: 14, borderRadius: '50%',
          background: hw.bgInput, border: `1px solid ${hw.border}`,
          position: 'relative',
        }}>
          <div style={{
            position: 'absolute', width: 1, height: 5, background: hw.textMuted,
            top: 2, left: '50%', transform: 'translateX(-50%)',
            borderRadius: 1,
          }} />
        </div>
      </div>

      <Sep />

      {/* 14. CPU / Polyphony meters */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 4 }} onMouseEnter={hint('CPU / Polyphony')} onMouseLeave={clear}>
        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 1 }}>
          <span style={{ fontSize: 7, color: hw.textFaint, lineHeight: 1 }}>CPU</span>
          <div style={{ width: 28, height: 3, background: hw.bgInput, borderRadius: 1 }}>
            <div style={{ width: '12%', height: '100%', background: hw.green, borderRadius: 1 }} />
          </div>
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 1 }}>
          <span style={{ fontSize: 7, color: hw.textFaint, lineHeight: 1 }}>POLY</span>
          <div style={{ width: 28, height: 3, background: hw.bgInput, borderRadius: 1 }}>
            <div style={{ width: '5%', height: '100%', background: hw.green, borderRadius: 1 }} />
          </div>
        </div>
      </div>

      <Sep />

      {/* 15. Mini scope / output monitor */}
      <div style={{
        width: 56, height: 20,
        background: hw.bgDeep, border: `1px solid ${hw.border}`,
        borderRadius: 2,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}>
        <span style={{ fontSize: 7, color: hw.textFaint }}>SCOPE</span>
      </div>
    </div>
  )
}

/* --- Sub-components --- */

function Sep() {
  return <div style={{ width: 1, height: 20, background: hw.border, margin: '0 3px' }} />
}

function ToolBtn({ children, onEnter, onLeave }: {
  children: React.ReactNode; onEnter: () => void; onLeave: () => void
}) {
  return (
    <button
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      style={{
        width: 22, height: 22,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        color: hw.textFaint,
        background: 'transparent',
        border: '1px solid transparent',
        borderRadius: 2,
      }}
    >
      {children}
    </button>
  )
}

function ToolSelectBtn({ tool, active, onClick, children, onEnter, onLeave }: {
  tool: Tool; active: Tool; onClick: (t: Tool) => void
  children: React.ReactNode; onEnter: () => void; onLeave: () => void
}) {
  const isActive = tool === active
  return (
    <button
      onClick={() => onClick(tool)}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      style={{
        width: 20, height: 20,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        color: isActive ? hw.purple : hw.textFaint,
        background: isActive ? hw.purpleDim : 'transparent',
        border: `1px solid ${isActive ? hw.purple + '40' : 'transparent'}`,
        borderRadius: 2,
      }}
    >
      {children}
    </button>
  )
}

function IconBtn({ icon, active, onClick, onEnter, onLeave }: {
  icon: string; active: boolean; onClick: () => void; onEnter: () => void; onLeave: () => void
}) {
  const icons: Record<string, React.ReactNode> = {
    playlist: <svg width="11" height="11" viewBox="0 0 11 11"><rect x="0.5" y="1" width="4" height="3" rx="0.5" fill="currentColor"/><rect x="5.5" y="1" width="5" height="3" rx="0.5" fill="currentColor" opacity="0.6"/><rect x="1.5" y="6" width="6" height="3" rx="0.5" fill="currentColor"/></svg>,
    channel: <svg width="11" height="11" viewBox="0 0 11 11"><rect x="0.5" y="1.5" width="10" height="2" rx="0.5" fill="currentColor"/><rect x="0.5" y="4.5" width="10" height="2" rx="0.5" fill="currentColor"/><rect x="0.5" y="7.5" width="10" height="2" rx="0.5" fill="currentColor"/></svg>,
    pianoroll: <svg width="11" height="11" viewBox="0 0 11 11"><rect x="0.5" y="0.5" width="10" height="10" rx="1" fill="none" stroke="currentColor" strokeWidth="0.8"/><rect x="0.5" y="0.5" width="3" height="2" fill="currentColor"/><rect x="0.5" y="4" width="3" height="2" fill="currentColor"/><rect x="0.5" y="7.5" width="3" height="2" fill="currentColor"/><rect x="5" y="3" width="5" height="1.5" rx="0.5" fill="currentColor" opacity="0.7"/><rect x="4" y="6" width="4" height="1.5" rx="0.5" fill="currentColor" opacity="0.7"/></svg>,
    mixer: <svg width="11" height="11" viewBox="0 0 11 11"><line x1="2.5" y1="1" x2="2.5" y2="10" stroke="currentColor" strokeWidth="1.2"/><line x1="5.5" y1="1" x2="5.5" y2="10" stroke="currentColor" strokeWidth="1.2"/><line x1="8.5" y1="1" x2="8.5" y2="10" stroke="currentColor" strokeWidth="1.2"/><circle cx="2.5" cy="3.5" r="1.3" fill="currentColor"/><circle cx="5.5" cy="6.5" r="1.3" fill="currentColor"/><circle cx="8.5" cy="5" r="1.3" fill="currentColor"/></svg>,
    browser: <svg width="11" height="11" viewBox="0 0 11 11"><rect x="0.5" y="0.5" width="10" height="10" rx="1" fill="none" stroke="currentColor" strokeWidth="1"/><line x1="3.5" y1="0.5" x2="3.5" y2="10.5" stroke="currentColor" strokeWidth="1"/></svg>,
  }
  return (
    <button
      onClick={onClick}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      style={{
        width: 22, height: 22,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        color: active ? hw.purple : hw.textFaint,
        background: active ? hw.purpleDim : 'transparent',
        border: `1px solid ${active ? hw.purple + '40' : 'transparent'}`,
        borderRadius: 2,
      }}
    >
      {icons[icon]}
    </button>
  )
}

function ModeBtn({ label, active, onEnter, onLeave }: {
  label: string; active: boolean; onEnter: () => void; onLeave: () => void
}) {
  return (
    <div
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      style={{
        padding: '2px 6px',
        fontSize: 9, fontWeight: 700,
        color: active ? hw.purple : hw.textFaint,
        background: active ? hw.bgCard : 'transparent',
        cursor: 'default',
        letterSpacing: 0.5,
      }}
    >
      {label}
    </div>
  )
}

const tBtn: React.CSSProperties = {
  width: 24, height: 22,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: '#222226',
  border: '1px solid rgba(255,255,255,0.07)',
  borderRadius: 2,
}

const patNavBtn: React.CSSProperties = {
  width: 14, height: 22,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: 'transparent',
  border: 'none',
  cursor: 'pointer',
}

const lcdBox: React.CSSProperties = {
  display: 'flex', alignItems: 'center',
  background: '#111114',
  border: '1px solid rgba(255,255,255,0.07)',
  borderRadius: 2,
  height: 22,
  padding: '0 3px',
}

const lcdText: React.CSSProperties = {
  fontFamily: "'Consolas', 'Courier New', monospace",
  fontSize: 12, fontWeight: 700,
  whiteSpace: 'pre',
  letterSpacing: 0,
}
