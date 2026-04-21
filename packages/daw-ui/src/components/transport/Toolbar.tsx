import { useEffect, useRef, useState } from 'react'
import { hw } from '../../theme'
import { useTransportStore, SNAP_VALUES } from '../../stores/transportStore'
import { useTrackStore } from '../../stores/trackStore'
import { usePatternStore } from '../../stores/patternStore'
import { useMetronomeStore } from '../../stores/metronomeStore'

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
  const {
    playing, looping, bpm, positionSamples, sampleRate,
    masterVolumeDb, timeSigNumerator, timeSigDenominator, patternMode,
    togglePlayback, stop, setBpm, toggleLoop, tapTempo,
    setMasterVolume, setTimeSignature, setPatternMode,
    snapValue, snapEnabled, setSnapValue, toggleSnap,
    horizontalZoom, setHorizontalZoom, zoomToFit,
  } = useTransportStore()
  const undoTracks = useTrackStore(s => s.undo)
  const redoTracks = useTrackStore(s => s.redo)
  const patterns = usePatternStore(s => s.patterns)
  const activePatternId = usePatternStore(s => s.activeId)
  const setActivePattern = usePatternStore(s => s.setActive)
  const prevPattern = usePatternStore(s => s.prevPattern)
  const nextPattern = usePatternStore(s => s.nextPattern)
  const activePattern = patterns.find(p => p.id === activePatternId) || patterns[0]
  const [activeTool, setActiveTool] = useState<Tool>('draw')

  const seconds = sampleRate > 0 ? positionSamples / sampleRate : 0
  const hrs = Math.floor(seconds / 3600)
  const mins = Math.floor((seconds % 3600) / 60)
  const secs = Math.floor(seconds % 60)
  const ms = Math.floor((seconds % 1) * 1000)
  const beats = bpm > 0 ? (seconds * bpm / 60) : 0
  const beatsPerBar = timeSigNumerator > 0 ? timeSigNumerator : 4
  const bar = Math.floor(beats / beatsPerBar) + 1
  const beat = Math.floor(beats % beatsPerBar) + 1
  const tick = Math.floor((beats % 1) * 960)

  const hint = (text: string) => () => props.onSetHint(text)
  const clear = () => props.onSetHint('')

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      height: 40,
      background: hw.bgToolbarGrad,
      backdropFilter: hw.blur.md,
      borderBottom: `1px solid ${hw.border}`,
      padding: '0 6px',
      gap: 2,
    }}>
      {/* 1. Panel toggle buttons */}
      <div style={{ display: 'flex', gap: 2 }}>
        <PanelBtn icon="playlist" label="Playlist" active={props.showPlaylist} onClick={props.onTogglePlaylist} onEnter={hint('Playlist (F5)')} onLeave={clear} />
        <PanelBtn icon="channel" label="Channel" active={props.showChannelRack} onClick={props.onToggleChannelRack} onEnter={hint('Channel Rack (F6)')} onLeave={clear} />
        <PanelBtn icon="pianoroll" label="Piano" active={props.showPianoRoll} onClick={props.onTogglePianoRoll} onEnter={hint('Piano Roll (F7)')} onLeave={clear} />
        <PanelBtn icon="mixer" label="Mixer" active={props.showMixer} onClick={props.onToggleMixer} onEnter={hint('Mixer (F9)')} onLeave={clear} />
        <PanelBtn icon="browser" label="Browser" active={props.showBrowser} onClick={props.onToggleBrowser} onEnter={hint('Browser')} onLeave={clear} />
      </div>

      <Sep />

      {/* 1b. Undo / Redo */}
      <div style={{ display: 'flex', gap: 2 }}>
        <ToolBtn onEnter={hint('Undo (Ctrl+Z)')} onLeave={clear} onClick={() => undoTracks()}>
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.1" strokeLinecap="round" strokeLinejoin="round">
            <polyline points="3.5 4.5 1.5 4.5 1.5 2.5" />
            <path d="M2 6.5a4 4 0 1 0 1.2-2.8L1.5 5.2" />
          </svg>
        </ToolBtn>
        <ToolBtn onEnter={hint('Redo (Ctrl+Y)')} onLeave={clear} onClick={() => redoTracks()}>
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.1" strokeLinecap="round" strokeLinejoin="round">
            <polyline points="8.5 4.5 10.5 4.5 10.5 2.5" />
            <path d="M10 6.5a4 4 0 1 1-1.2-2.8L10.5 5.2" />
          </svg>
        </ToolBtn>
      </div>

      <Sep />

      {/* 2. Pattern selector */}
      <div style={{ display: 'flex', alignItems: 'center' }}>
        <button onClick={prevPattern} style={navBtn} onMouseEnter={hint('Previous pattern')} onMouseLeave={clear}>
          <svg width="5" height="7" viewBox="0 0 5 7"><path d="M4 0.5L1 3.5L4 6.5" stroke={hw.textMuted} strokeWidth="1.2" fill="none"/></svg>
        </button>
        <div style={{
          ...lcd, padding: '0 4px', minWidth: 80,
        }} onMouseEnter={hint('Select pattern')} onMouseLeave={clear}>
          <select
            value={activePattern.id}
            onChange={(e) => setActivePattern(e.target.value)}
            data-testid="toolbar-pattern-select"
            style={{
              background: 'transparent', border: 'none', color: hw.textSecondary,
              fontSize: 10, outline: 'none', appearance: 'none', cursor: 'pointer',
              width: '100%', fontFamily: "'Consolas', monospace",
            }}
          >
            {patterns.map(p => (
              <option key={p.id} value={p.id}>{p.name}</option>
            ))}
          </select>
        </div>
        <button onClick={nextPattern} style={navBtn} onMouseEnter={hint('Next pattern')} onMouseLeave={clear}>
          <svg width="5" height="7" viewBox="0 0 5 7"><path d="M1 0.5L4 3.5L1 6.5" stroke={hw.textMuted} strokeWidth="1.2" fill="none"/></svg>
        </button>
      </div>

      <Sep />

      {/* 3. PAT / SONG toggle */}
      <div style={{
        display: 'flex', background: 'rgba(255,255,255,0.03)',
        border: `1px solid ${hw.borderDark}`,
        overflow: 'hidden', borderRadius: hw.radius.sm,
      }}>
        <ModeBtn label="PAT" active={patternMode} onClick={() => setPatternMode(true)} onEnter={hint('Pattern mode')} onLeave={clear} />
        <div style={{ width: 1, background: hw.borderDark }} />
        <ModeBtn label="SONG" active={!patternMode} onClick={() => setPatternMode(false)} onEnter={hint('Song mode')} onLeave={clear} />
      </div>

      <Sep />

      {/* 4. Transport — Record, Stop, Play */}
      <div style={{ display: 'flex', gap: 2 }}>
        <button style={transportBtn} onMouseEnter={hint('Record (R)')} onMouseLeave={clear}>
          <svg width="10" height="10"><circle cx="5" cy="5" r="4" fill={hw.red} opacity="0.7" /></svg>
        </button>
        <button onClick={stop} style={transportBtn} onMouseEnter={hint('Stop')} onMouseLeave={clear}>
          <svg width="10" height="10"><rect x="1" y="1" width="8" height="8" rx="1" fill={hw.textMuted} /></svg>
        </button>
        <button onClick={togglePlayback} style={{
          ...transportBtn,
          background: playing ? hw.accentDim : transportBtn.background,
          borderColor: playing ? hw.accentGlow : 'rgba(255,255,255,0.06)',
        }} onMouseEnter={hint('Play (Space)')} onMouseLeave={clear}>
          <svg width="10" height="12"><polygon points="0,0 10,6 0,12" fill={playing ? hw.accent : hw.textMuted} /></svg>
        </button>
        <button onClick={toggleLoop} style={{
          ...transportBtn,
          background: looping ? 'rgba(234,179,8,0.15)' : transportBtn.background,
          borderColor: looping ? 'rgba(234,179,8,0.3)' : 'rgba(255,255,255,0.06)',
        }} onMouseEnter={hint('Loop (L)')} onMouseLeave={clear}>
          <svg width="12" height="10" viewBox="0 0 12 10">
            <path d="M3 1h6a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V3a2 2 0 0 1 2-2z" fill="none" stroke={looping ? '#eab308' : hw.textMuted} strokeWidth="1.2"/>
            <path d="M8 1l2 1.5L8 4" fill="none" stroke={looping ? '#eab308' : hw.textMuted} strokeWidth="1"/>
          </svg>
        </button>
        <MetronomeButton onEnter={hint} onLeave={clear} />
      </div>

      <Sep />

      {/* 5. Tempo LCD + Tap */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 2 }}>
        <div style={{ ...lcd, width: 68 }} onMouseEnter={hint('Tempo')} onMouseLeave={clear}>
          <input
            type="number" value={bpm}
            onChange={e => setBpm(parseFloat(e.target.value) || 140)}
            style={{
              width: 50, background: 'transparent', border: 'none',
              color: hw.textPrimary, fontSize: 14, fontWeight: 700,
              fontFamily: "'Consolas', 'Courier New', monospace",
              textAlign: 'right', outline: 'none',
            }}
          />
        </div>
        <button onClick={tapTempo} style={{
          ...transportBtn, width: 24, height: 24, fontSize: 8, fontWeight: 700,
          color: hw.textMuted, letterSpacing: 0.3,
        }} onMouseEnter={hint('Tap tempo')} onMouseLeave={clear}>
          TAP
        </button>
        <div style={{ ...lcd, padding: '0 4px', gap: 1 }} onMouseEnter={hint('Time signature')} onMouseLeave={clear}>
          <input
            type="number" min={1} max={32} value={timeSigNumerator}
            onChange={e => setTimeSignature(Math.max(1, parseInt(e.target.value) || 4), timeSigDenominator)}
            style={{
              width: 20, background: 'transparent', border: 'none',
              color: hw.textPrimary, fontSize: 11, fontWeight: 700,
              fontFamily: "'Consolas', 'Courier New', monospace",
              textAlign: 'center', outline: 'none',
            }}
          />
          <span style={{ color: hw.textFaint, fontSize: 11 }}>/</span>
          <select
            value={timeSigDenominator}
            onChange={e => setTimeSignature(timeSigNumerator, parseInt(e.target.value))}
            style={{
              background: 'transparent', border: 'none',
              color: hw.textPrimary, fontSize: 11, fontWeight: 700,
              fontFamily: "'Consolas', 'Courier New', monospace",
              outline: 'none', appearance: 'none', width: 20, textAlign: 'center',
            }}
          >
            {[1, 2, 4, 8, 16, 32].map(d => <option key={d} value={d}>{d}</option>)}
          </select>
        </div>
      </div>

      <Sep />

      {/* 6. Time display — Bar:Beat:Tick | Min:Sec.Cs */}
      <div style={{ ...lcd, padding: '0 8px', gap: 8 }}>
        <span style={lcdDigit}>
          <span style={{ color: hw.textPrimary }}>{String(bar).padStart(3, ' ')}</span>
          <span style={{ color: hw.textFaint }}>:</span>
          <span style={{ color: hw.textPrimary }}>{beat}</span>
          <span style={{ color: hw.textFaint }}>:</span>
          <span style={{ color: hw.textPrimary }}>{String(tick).padStart(3, '0')}</span>
        </span>
        <div style={{ width: 1, height: 14, background: hw.border }} />
        <span style={lcdDigit}>
          <span style={{ color: hw.textPrimary }}>{hrs}</span>
          <span style={{ color: hw.textFaint }}>:</span>
          <span style={{ color: hw.textPrimary }}>{String(mins).padStart(2, '0')}</span>
          <span style={{ color: hw.textFaint }}>:</span>
          <span style={{ color: hw.textPrimary }}>{String(secs).padStart(2, '0')}</span>
          <span style={{ color: hw.textFaint }}>.</span>
          <span style={{ color: hw.textPrimary }}>{String(ms).padStart(3, '0')}</span>
        </span>
      </div>

      <Sep />

      {/* 7. Song position slider */}
      <div style={{
        width: 80, height: 8, background: 'rgba(255,255,255,0.04)', borderRadius: hw.radius.sm,
        border: `1px solid ${hw.borderDark}`, position: 'relative', cursor: 'pointer',
      }} onMouseEnter={hint('Song position')} onMouseLeave={clear}>
        <div style={{
          position: 'absolute', left: 0, top: 0, width: '0%', height: '100%',
          background: `linear-gradient(90deg, ${hw.secondary}, ${hw.accent})`,
          borderRadius: hw.radius.sm,
          opacity: 0.5,
        }} />
      </div>

      <Sep />

      {/* 8. Snap */}
      <div style={{ ...lcd, padding: '0 4px', gap: 3 }} onMouseEnter={hint(`Snap: ${snapEnabled ? snapValue : 'Off'}`)} onMouseLeave={clear}>
        <button
          onClick={toggleSnap}
          title="Toggle snap"
          style={{
            width: 18, height: 18, display: 'flex', alignItems: 'center', justifyContent: 'center',
            background: snapEnabled ? hw.accentDim : 'transparent',
            border: `1px solid ${snapEnabled ? hw.accentGlow : 'transparent'}`,
            borderRadius: hw.radius.sm, color: snapEnabled ? hw.accent : hw.textMuted, padding: 0,
          }}
        >
          <svg width="8" height="8" viewBox="0 0 8 8">
            <path d="M1 7V1h6" stroke="currentColor" strokeWidth="0.9" fill="none"/>
            <circle cx="4" cy="4" r="1.3" fill="currentColor"/>
          </svg>
        </button>
        <select
          value={snapValue}
          onChange={e => setSnapValue(e.target.value as any)}
          data-testid="snap-select"
          style={{
            background: 'transparent', border: 'none', color: hw.textSecondary,
            fontSize: 10, fontFamily: "'Consolas', 'Courier New', monospace", outline: 'none',
            padding: '0 2px', appearance: 'none', cursor: 'pointer', minWidth: 38,
          }}
        >
          {SNAP_VALUES.map(v => <option key={v} value={v}>{v}</option>)}
        </select>
      </div>

      <Sep />

      {/* 8b. Horizontal zoom */}
      <div style={{ ...lcd, padding: '0 2px', gap: 1 }} onMouseEnter={hint(`Zoom: ${horizontalZoom.toFixed(2)}x`)} onMouseLeave={clear}>
        <button onClick={() => setHorizontalZoom(horizontalZoom / 1.25)} style={zoomBtn} title="Zoom out">−</button>
        <button onClick={zoomToFit} data-testid="zoom-to-fit" style={{ ...zoomBtn, fontSize: 8, letterSpacing: 0.3 }} title="Zoom to fit">FIT</button>
        <button onClick={() => setHorizontalZoom(horizontalZoom * 1.25)} style={zoomBtn} title="Zoom in">+</button>
      </div>

      <Sep />

      {/* 9. Tool buttons */}
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

      {/* 10. Multilink */}
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
      <div style={{ display: 'flex', alignItems: 'center', gap: 3 }} onMouseEnter={hint(`Master volume: ${masterVolumeDb.toFixed(1)} dB`)} onMouseLeave={clear}>
        <svg width="9" height="9" viewBox="0 0 9 9">
          <polygon points="0,4 3,1 3,7" fill={hw.textMuted} />
          <path d="M4.5 2.5 Q6 4.5 4.5 6.5" stroke={hw.textMuted} strokeWidth="0.8" fill="none"/>
          <path d="M5.5 1.5 Q8 4.5 5.5 7.5" stroke={hw.textMuted} strokeWidth="0.8" fill="none"/>
        </svg>
        <MasterVolumeSlider valueDb={masterVolumeDb} onChange={setMasterVolume} />
      </div>

      <Sep />

      {/* 13. Master pitch knob */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 2 }} onMouseEnter={hint('Master pitch')} onMouseLeave={clear}>
        <span style={{ fontSize: 7, color: hw.textFaint }}>PIT</span>
        <div style={{
          width: 16, height: 16, borderRadius: '50%',
          background: 'rgba(255,255,255,0.04)',
          border: `1px solid ${hw.borderDark}`,
          position: 'relative',
        }}>
          <div style={{
            position: 'absolute', width: 1.5, height: 5, background: hw.accent,
            top: 2, left: '50%', transform: 'translateX(-50%)',
            borderRadius: 1,
          }} />
        </div>
      </div>

      <Sep />

      {/* 14. CPU / Polyphony */}
      <CpuPolyMeters onEnter={hint} onLeave={clear} />

      <Sep />

      {/* 15. Mini scope */}
      <div style={{
        width: 60, height: 24,
        background: 'rgba(255,255,255,0.03)', border: `1px solid ${hw.borderDark}`,
        borderRadius: hw.radius.sm,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}>
        <svg width="40" height="12" viewBox="0 0 40 12">
          <polyline points="0,6 5,6 8,3 12,9 16,4 20,8 24,5 28,7 32,6 36,6 40,6"
            fill="none" stroke={hw.accent} strokeWidth="1" opacity="0.5" />
        </svg>
      </div>
    </div>
  )
}

/* --- Sub-components --- */

// Frame-time based CPU approximation. Samples the interval between rAF callbacks
// and compares it to a 16.67ms target (60fps). A slowdown below 60fps indicates
// either the audio thread or other main-thread work is eating CPU.
function useCpuEstimate(): number {
  const [pct, setPct] = useState(0)
  const samples = useRef<number[]>([])
  const lastTs = useRef<number>(0)
  const rafId = useRef<number>(0)
  const lastUpdate = useRef<number>(0)
  useEffect(() => {
    const TARGET_MS = 1000 / 60
    const tick = (ts: number) => {
      if (lastTs.current > 0) {
        const dt = ts - lastTs.current
        samples.current.push(dt)
        if (samples.current.length > 60) samples.current.shift()
      }
      lastTs.current = ts
      if (ts - lastUpdate.current > 500) {
        lastUpdate.current = ts
        const arr = samples.current
        if (arr.length > 10) {
          const avg = arr.reduce((a, b) => a + b, 0) / arr.length
          const load = Math.max(0, Math.min(100, Math.round((1 - TARGET_MS / avg) * 100)))
          setPct(load)
        }
      }
      rafId.current = requestAnimationFrame(tick)
    }
    rafId.current = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(rafId.current)
  }, [])
  return pct
}

const POLY_MAX = 32
const PPQ = 960

// Active-clip count as a polyphony proxy. Counts clips whose
// [position_ticks, position_ticks + length_ticks) contains the playhead,
// across every non-master track.
function usePolyphonyEstimate(): number {
  const playing = useTransportStore(s => s.playing)
  const positionSamples = useTransportStore(s => s.positionSamples)
  const sampleRate = useTransportStore(s => s.sampleRate)
  const bpm = useTransportStore(s => s.bpm)
  const tracks = useTrackStore(s => s.tracks)
  if (!playing) return 0
  const sr = sampleRate || 48000
  const tick = Math.round((positionSamples / sr) * (bpm / 60) * PPQ)
  let count = 0
  for (const t of tracks) {
    if (t.kind === 'Master') continue
    for (const c of t.clips) {
      if (tick >= c.position_ticks && tick < c.position_ticks + c.length_ticks) count++
    }
  }
  return count
}

function CpuPolyMeters({ onEnter, onLeave }: { onEnter: (text: string) => () => void; onLeave: () => void }) {
  const cpu = useCpuEstimate()
  const poly = usePolyphonyEstimate()
  const cpuColor = cpu > 80 ? hw.red : cpu > 50 ? hw.yellow : hw.green
  const polyPct = Math.max(2, Math.min(100, Math.round((poly / POLY_MAX) * 100)))
  const polyColor = poly > POLY_MAX * 0.75 ? hw.red : poly > POLY_MAX * 0.5 ? hw.yellow : hw.green
  return (
    <div
      style={{ display: 'flex', alignItems: 'center', gap: 4 }}
      onMouseEnter={onEnter(`CPU ${cpu}% · Polyphony ${poly}`)}
      onMouseLeave={onLeave}
    >
      <div
        data-testid="toolbar-cpu"
        title={`CPU load ~${cpu}% (frame-time estimate)`}
        style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 1 }}
      >
        <span style={{ fontSize: 7, color: hw.textFaint, lineHeight: 1 }}>CPU</span>
        <div style={{ width: 30, height: 4, background: 'rgba(255,255,255,0.04)', borderRadius: hw.radius.sm, border: `1px solid ${hw.borderDark}` }}>
          <div style={{
            width: `${cpu}%`, height: '100%', background: cpuColor,
            borderRadius: hw.radius.sm, transition: 'width 0.3s, background 0.3s',
          }} />
        </div>
      </div>
      <div
        data-testid="toolbar-poly"
        title={`${poly} active clip${poly === 1 ? '' : 's'} at playhead`}
        style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 1 }}
      >
        <span style={{ fontSize: 7, color: hw.textFaint, lineHeight: 1 }}>POLY {poly}</span>
        <div style={{ width: 30, height: 4, background: 'rgba(255,255,255,0.04)', borderRadius: hw.radius.sm, border: `1px solid ${hw.borderDark}` }}>
          <div style={{
            width: `${polyPct}%`, height: '100%', background: polyColor,
            borderRadius: hw.radius.sm, transition: 'width 0.2s, background 0.3s',
          }} />
        </div>
      </div>
    </div>
  )
}

function MetronomeButton({ onEnter, onLeave }: {
  onEnter: (text: string) => () => void; onLeave: () => void
}) {
  const enabled = useMetronomeStore(s => s.enabled)
  const volume = useMetronomeStore(s => s.volume)
  const accent = useMetronomeStore(s => s.accent)
  const recordOnly = useMetronomeStore(s => s.recordOnly)
  const precountBars = useMetronomeStore(s => s.precountBars)
  const customDownbeatName = useMetronomeStore(s => s.customDownbeatName)
  const customAccentName = useMetronomeStore(s => s.customAccentName)
  const toggleEnabled = useMetronomeStore(s => s.toggleEnabled)
  const setVolume = useMetronomeStore(s => s.setVolume)
  const setAccent = useMetronomeStore(s => s.setAccent)
  const setRecordOnly = useMetronomeStore(s => s.setRecordOnly)
  const setPrecountBars = useMetronomeStore(s => s.setPrecountBars)
  const setCustomDownbeat = useMetronomeStore(s => s.setCustomDownbeat)
  const setCustomAccent = useMetronomeStore(s => s.setCustomAccent)
  const [open, setOpen] = useState(false)
  const downInputRef = useRef<HTMLInputElement | null>(null)
  const accentInputRef = useRef<HTMLInputElement | null>(null)

  const readAsDataUrl = (file: File, apply: (dataUrl: string, name: string) => void) => {
    const reader = new FileReader()
    reader.onload = () => {
      const dataUrl = reader.result
      if (typeof dataUrl === 'string') apply(dataUrl, file.name)
    }
    reader.readAsDataURL(file)
  }
  useEffect(() => {
    if (!open) return
    const close = () => setOpen(false)
    window.addEventListener('click', close)
    return () => window.removeEventListener('click', close)
  }, [open])

  return (
    <div style={{ position: 'relative' }}
      onMouseEnter={onEnter(enabled ? `Metronome ON — volume ${Math.round(volume * 100)}%` : 'Metronome (click to enable, right-click for options)')}
      onMouseLeave={onLeave}
    >
      <button
        onClick={toggleEnabled}
        onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); setOpen(v => !v) }}
        style={{
          ...transportBtn,
          background: enabled ? hw.accentDim : transportBtn.background,
          borderColor: enabled ? hw.accentGlow : 'rgba(255,255,255,0.06)',
        }}
      >
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke={enabled ? hw.accent : hw.textMuted} strokeWidth="1.1" strokeLinejoin="round">
          <path d="M3.5 10.5L5 1.5h2l1.5 9z" />
          <line x1="2.5" y1="10.5" x2="9.5" y2="10.5" />
          <line x1="6" y1="6" x2="9.5" y2="3" />
        </svg>
      </button>
      {open && (
        <div
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
          style={{
            position: 'absolute', top: 30, left: 0, zIndex: 500,
            minWidth: 200, padding: 10,
            background: 'rgba(12,12,18,0.96)',
            border: `1px solid ${hw.borderLight}`,
            borderRadius: hw.radius.md,
            boxShadow: '0 8px 32px rgba(0,0,0,0.55)',
            backdropFilter: hw.blur.md,
            display: 'flex', flexDirection: 'column', gap: 8,
          }}
        >
          <div style={{ fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
            Metronome
          </div>
          <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 10, color: hw.textMuted }}>
            <input type="checkbox" checked={enabled} onChange={(e) => useMetronomeStore.getState().setEnabled(e.target.checked)} />
            Enable
          </label>
          <div>
            <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: hw.textMuted }}>
              <span>Volume</span>
              <span style={{ color: hw.textPrimary, fontWeight: 600 }}>{Math.round(volume * 100)}%</span>
            </div>
            <input type="range" min={0} max={100} step={1} value={Math.round(volume * 100)}
              onChange={(e) => setVolume(Number(e.target.value) / 100)}
              style={{ width: '100%', accentColor: hw.accent }} />
          </div>
          <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 10, color: hw.textMuted }}>
            <input type="checkbox" checked={accent} onChange={(e) => setAccent(e.target.checked)} />
            Accent beat 1 (higher pitch)
          </label>
          <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 10, color: hw.textMuted }}>
            <input type="checkbox" checked={recordOnly} onChange={(e) => setRecordOnly(e.target.checked)} />
            Only during recording
          </label>
          <div>
            <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: hw.textMuted }}>
              <span>Pre-count</span>
              <span style={{ color: hw.textPrimary, fontWeight: 600 }}>
                {precountBars === 0 ? 'off' : `${precountBars} bar${precountBars === 1 ? '' : 's'}`}
              </span>
            </div>
            <div style={{ display: 'flex', gap: 2 }}>
              {[0, 1, 2, 4].map(n => (
                <button key={n}
                  onClick={() => setPrecountBars(n)}
                  style={{
                    flex: 1, padding: '3px 0', fontSize: 9, fontWeight: 600,
                    color: precountBars === n ? hw.accent : hw.textFaint,
                    background: precountBars === n ? hw.accentDim : 'transparent',
                    border: `1px solid ${precountBars === n ? hw.accentGlow : hw.border}`,
                    borderRadius: hw.radius.sm, cursor: 'pointer',
                  }}>
                  {n === 0 ? 'off' : `${n}b`}
                </button>
              ))}
            </div>
          </div>
          <div style={{ borderTop: `1px solid ${hw.border}`, paddingTop: 8, display: 'flex', flexDirection: 'column', gap: 6 }}>
            <div style={{ fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
              Custom samples
            </div>
            <SampleSlot
              label="Beat"
              name={customDownbeatName}
              onPick={() => downInputRef.current?.click()}
              onClear={() => setCustomDownbeat(null, null)}
            />
            <SampleSlot
              label="Accent"
              name={customAccentName}
              fallbackHint={customDownbeatName ? '(uses beat sample)' : '(uses sine tone)'}
              onPick={() => accentInputRef.current?.click()}
              onClear={() => setCustomAccent(null, null)}
            />
            <input
              ref={downInputRef}
              type="file"
              accept=".wav,.mp3,.ogg,audio/*"
              style={{ display: 'none' }}
              onChange={(e) => {
                const f = e.target.files?.[0]
                if (f) readAsDataUrl(f, (url, name) => setCustomDownbeat(url, name))
                e.target.value = ''
              }}
            />
            <input
              ref={accentInputRef}
              type="file"
              accept=".wav,.mp3,.ogg,audio/*"
              style={{ display: 'none' }}
              onChange={(e) => {
                const f = e.target.files?.[0]
                if (f) readAsDataUrl(f, (url, name) => setCustomAccent(url, name))
                e.target.value = ''
              }}
            />
          </div>
        </div>
      )}
    </div>
  )
}

function SampleSlot({ label, name, fallbackHint, onPick, onClear }: {
  label: string
  name: string | null
  fallbackHint?: string
  onPick: () => void
  onClear: () => void
}) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 9 }}>
      <span style={{ width: 54, color: hw.textMuted }}>{label}</span>
      <div
        title={name ?? undefined}
        style={{
          flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          color: name ? hw.textPrimary : hw.textFaint,
          fontStyle: name ? 'normal' : 'italic',
        }}
      >
        {name ?? (fallbackHint ?? 'default tone')}
      </div>
      <button
        onClick={onPick}
        style={{
          padding: '2px 6px', fontSize: 9, fontWeight: 600,
          color: hw.textMuted, background: 'transparent',
          border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, cursor: 'pointer',
        }}
      >
        Load…
      </button>
      {name && (
        <button
          onClick={onClear}
          title="Remove sample"
          style={{
            width: 18, padding: '2px 0', fontSize: 10, fontWeight: 700,
            color: hw.textFaint, background: 'transparent',
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, cursor: 'pointer',
          }}
        >
          ×
        </button>
      )}
    </div>
  )
}

function Sep() {
  return <div style={{ width: 1, height: 26, background: hw.border, margin: '0 3px' }} />
}

function ToolBtn({ children, onEnter, onLeave, onClick }: {
  children: React.ReactNode; onEnter: () => void; onLeave: () => void; onClick?: () => void
}) {
  return (
    <button
      onClick={onClick}
      onMouseEnter={e => { onEnter(); e.currentTarget.style.background = 'rgba(255,255,255,0.08)' }}
      onMouseLeave={e => { onLeave(); e.currentTarget.style.background = 'transparent' }}
      style={{
        width: 24, height: 24,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        color: hw.textMuted,
        background: 'transparent',
        border: '1px solid transparent',
        borderRadius: hw.radius.sm,
        transition: 'background 0.15s',
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
        width: 22, height: 22,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        color: isActive ? hw.accent : hw.textMuted,
        background: isActive ? hw.accentDim : 'transparent',
        border: `1px solid ${isActive ? hw.accentGlow : 'transparent'}`,
        borderRadius: hw.radius.sm,
        transition: 'all 0.1s',
      }}
    >
      {children}
    </button>
  )
}

function PanelBtn({ icon, active, onClick, onEnter, onLeave }: {
  icon: string; label: string; active: boolean; onClick: () => void; onEnter: () => void; onLeave: () => void
}) {
  const icons: Record<string, React.ReactNode> = {
    playlist: <svg width="12" height="12" viewBox="0 0 12 12"><rect x="0.5" y="1" width="4.5" height="3.5" rx="0.5" fill="currentColor"/><rect x="6" y="1" width="5.5" height="3.5" rx="0.5" fill="currentColor" opacity="0.6"/><rect x="1.5" y="6.5" width="7" height="3.5" rx="0.5" fill="currentColor"/></svg>,
    channel: <svg width="12" height="12" viewBox="0 0 12 12"><rect x="0.5" y="1.5" width="11" height="2.5" rx="0.5" fill="currentColor"/><rect x="0.5" y="5" width="11" height="2.5" rx="0.5" fill="currentColor"/><rect x="0.5" y="8.5" width="11" height="2.5" rx="0.5" fill="currentColor"/></svg>,
    pianoroll: <svg width="12" height="12" viewBox="0 0 12 12"><rect x="0.5" y="0.5" width="11" height="11" rx="1" fill="none" stroke="currentColor" strokeWidth="0.8"/><rect x="0.5" y="0.5" width="3.5" height="2.5" fill="currentColor"/><rect x="0.5" y="4.5" width="3.5" height="2.5" fill="currentColor"/><rect x="0.5" y="8.5" width="3.5" height="2.5" fill="currentColor"/><rect x="5.5" y="3.5" width="5" height="2" rx="0.5" fill="currentColor" opacity="0.7"/><rect x="4.5" y="7" width="4.5" height="2" rx="0.5" fill="currentColor" opacity="0.7"/></svg>,
    mixer: <svg width="12" height="12" viewBox="0 0 12 12"><line x1="3" y1="1" x2="3" y2="11" stroke="currentColor" strokeWidth="1.5"/><line x1="6" y1="1" x2="6" y2="11" stroke="currentColor" strokeWidth="1.5"/><line x1="9" y1="1" x2="9" y2="11" stroke="currentColor" strokeWidth="1.5"/><circle cx="3" cy="4" r="1.5" fill="currentColor"/><circle cx="6" cy="7" r="1.5" fill="currentColor"/><circle cx="9" cy="5.5" r="1.5" fill="currentColor"/></svg>,
    browser: <svg width="12" height="12" viewBox="0 0 12 12"><rect x="0.5" y="0.5" width="11" height="11" rx="1" fill="none" stroke="currentColor" strokeWidth="1"/><line x1="4" y1="0.5" x2="4" y2="11.5" stroke="currentColor" strokeWidth="1"/></svg>,
  }
  return (
    <button
      onClick={onClick}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      style={{
        width: 26, height: 26,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        color: active ? hw.accent : hw.textFaint,
        background: active ? hw.accentDim : 'transparent',
        border: `1px solid ${active ? hw.accentGlow : 'transparent'}`,
        borderRadius: hw.radius.sm,
        transition: 'all 0.1s',
      }}
    >
      {icons[icon]}
    </button>
  )
}

function ModeBtn({ label, active, onClick, onEnter, onLeave }: {
  label: string; active: boolean; onClick?: () => void; onEnter: () => void; onLeave: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      style={{
        padding: '3px 8px',
        fontSize: 10, fontWeight: 700,
        color: active ? hw.textPrimary : hw.textFaint,
        background: active ? 'rgba(255,255,255,0.04)' : 'transparent',
        border: 'none',
        cursor: onClick ? 'pointer' : 'default',
        letterSpacing: 0.5,
      }}
    >
      {label}
    </button>
  )
}

// Master volume slider: -60 dB .. +6 dB, drag-to-set, double-click to reset to 0 dB.
const MASTER_MIN_DB = -60
const MASTER_MAX_DB = 6
function MasterVolumeSlider({ valueDb, onChange }: { valueDb: number; onChange: (db: number) => void }) {
  const pct = Math.max(0, Math.min(1, (valueDb - MASTER_MIN_DB) / (MASTER_MAX_DB - MASTER_MIN_DB)))

  const handleDrag = (e: React.MouseEvent<HTMLDivElement>) => {
    const el = e.currentTarget
    const rect = el.getBoundingClientRect()
    const update = (clientX: number) => {
      const p = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width))
      const db = MASTER_MIN_DB + p * (MASTER_MAX_DB - MASTER_MIN_DB)
      onChange(db)
    }
    update(e.clientX)
    const move = (ev: MouseEvent) => update(ev.clientX)
    const up = () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
    }
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
  }

  return (
    <div
      onMouseDown={handleDrag}
      onDoubleClick={() => onChange(0)}
      style={{
        width: 55, height: 6, background: 'rgba(255,255,255,0.04)', borderRadius: hw.radius.sm,
        border: `1px solid ${hw.borderDark}`, position: 'relative', cursor: 'ew-resize',
      }}
    >
      <div style={{
        width: `${pct * 100}%`, height: '100%', borderRadius: hw.radius.sm,
        background: `linear-gradient(90deg, ${hw.secondary}, ${hw.accent})`,
        opacity: 0.75,
      }} />
    </div>
  )
}

const transportBtn: React.CSSProperties = {
  width: 28, height: 26,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: 'rgba(255,255,255,0.03)',
  border: `1px solid rgba(255,255,255,0.06)`,
  borderRadius: 6,
}

const navBtn: React.CSSProperties = {
  width: 16, height: 26,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: 'transparent',
  border: 'none',
  cursor: 'pointer',
}

const lcd: React.CSSProperties = {
  display: 'flex', alignItems: 'center',
  background: 'rgba(255,255,255,0.03)',
  border: `1px solid rgba(255,255,255,0.04)`,
  borderRadius: 6,
  height: 26,
  padding: '0 4px',
}

const zoomBtn: React.CSSProperties = {
  width: 18, height: 20, display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: 'transparent', border: 'none', color: '#a1a1aa',
  fontSize: 11, fontWeight: 700, fontFamily: "'Consolas', 'Courier New', monospace",
  cursor: 'pointer', padding: 0,
}

const lcdDigit: React.CSSProperties = {
  fontFamily: "'Consolas', 'Courier New', monospace",
  fontSize: 13, fontWeight: 700,
  whiteSpace: 'pre',
  letterSpacing: 0,
}
