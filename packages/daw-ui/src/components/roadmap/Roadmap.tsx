import { useState } from 'react'
import { hw } from '../../theme'

type Phase = 'current' | 'phase1' | 'phase2' | 'phase3' | 'phase4' | 'phase5' | 'stretch'

interface Milestone {
  name: string
  description: string
  features: { name: string; done: boolean; detail?: string }[]
}

const ROADMAP: Record<Phase, { title: string; subtitle: string; color: string; milestones: Milestone[] }> = {
  current: {
    title: 'v0.1.x — Current Release',
    subtitle: 'Foundation & basic audio playback',
    color: hw.accent,
    milestones: [
      {
        name: 'Core Audio Engine',
        description: 'Rust audio graph with real-time processing',
        features: [
          { name: 'Audio device output (cpal)', done: true },
          { name: 'Audio file decode (symphonia)', done: true },
          { name: 'Track audio graph with summing', done: true },
          { name: 'Master bus with volume', done: true },
          { name: 'Per-track volume & pan', done: true },
          { name: 'Mute / Solo per track', done: true },
          { name: 'Sample-accurate transport', done: true },
          { name: 'Peak metering (master)', done: true },
        ],
      },
      {
        name: 'Playlist / Arrangement',
        description: 'Timeline with audio clips',
        features: [
          { name: 'Audio clip placement on timeline', done: true },
          { name: 'Waveform rendering in clips', done: true },
          { name: 'Clip move (drag)', done: true },
          { name: 'Clip resize (drag edges)', done: true },
          { name: 'Clip delete', done: true },
          { name: 'Playhead with position tracking', done: true },
          { name: 'Beat grid / ruler', done: true },
          { name: 'Audio file import (dialog)', done: true },
        ],
      },
      {
        name: 'Hardwave UI',
        description: 'Dark glassmorphic theme with red accents, panel system',
        features: [
          { name: 'Hardwave dark theme', done: true },
          { name: 'Title bar with menu items', done: true },
          { name: 'Toolbar with transport, PAT/SONG, time display', done: true },
          { name: 'Browser panel (plugins/files/project tabs)', done: true },
          { name: 'Channel Rack with step sequencer', done: true },
          { name: 'Mixer with vertical faders & meters', done: true },
          { name: 'Panel toggles (F5/F6/F9)', done: true },
          { name: 'Auto-updater (GitHub releases)', done: true },
        ],
      },
    ],
  },
  phase1: {
    title: 'v0.2 — Core Editing',
    subtitle: 'Piano roll, MIDI, recording, project save/load',
    color: '#4488FF',
    milestones: [
      {
        name: 'Project Format (.hwp)',
        description: 'Native project save/load with full state serialization',
        features: [
          { name: 'Save project to .hwp (JSON + audio assets)', done: false, detail: 'Serialize tracks, clips, mixer state, plugin chains, automation — zip with referenced audio files' },
          { name: 'Load .hwp projects', done: false, detail: 'Deserialize and rebuild engine state, re-link audio pool' },
          { name: 'Auto-save with recovery', done: false, detail: 'Timed backups, crash recovery dialog on launch' },
          { name: 'Recent projects list', done: false },
          { name: 'Project templates', done: false, detail: 'Blank, 4-track, beat template, vocal session' },
          { name: 'Export stems (per-track render)', done: false },
        ],
      },
      {
        name: 'Piano Roll',
        description: 'MIDI note editor',
        features: [
          { name: 'Note grid with draw/select/delete tools', done: false },
          { name: 'Note velocity editing (bottom lane)', done: false },
          { name: 'Note resize and move', done: false },
          { name: 'Snap to grid (beat subdivisions)', done: false },
          { name: 'Scale highlighting & snapping', done: false },
          { name: 'Ghost notes (other channels in background)', done: false },
          { name: 'Chord stamp tool', done: false },
          { name: 'Quantize with strength %', done: false },
          { name: 'Copy / paste / duplicate', done: false },
          { name: 'Zoom & scroll', done: false },
          { name: 'MIDI CC lanes (pitch bend, mod wheel, etc.)', done: false },
          { name: 'Keyboard input (QWERTY to MIDI)', done: false },
        ],
      },
      {
        name: 'Audio Recording',
        description: 'Record audio input to tracks',
        features: [
          { name: 'Audio input device selection', done: false },
          { name: 'Track arming for record', done: false },
          { name: 'Record to playlist track', done: false },
          { name: 'Input monitoring with FX', done: false },
          { name: 'Loop recording (overdub / replace)', done: false },
          { name: 'Latency compensation on recorded audio', done: false },
          { name: 'Pre-count / metronome click', done: false },
        ],
      },
      {
        name: 'MIDI Recording',
        description: 'Record MIDI input to piano roll',
        features: [
          { name: 'MIDI input device selection', done: false },
          { name: 'Record MIDI to piano roll', done: false },
          { name: 'Step recording mode', done: false },
          { name: 'MIDI CC recording', done: false },
          { name: 'Quantize on input', done: false },
        ],
      },
    ],
  },
  phase2: {
    title: 'v0.3 — Mixing & Automation',
    subtitle: 'Professional mixing tools and automation system',
    color: hw.green,
    milestones: [
      {
        name: 'Automation System',
        description: 'Drawable automation clips for any parameter',
        features: [
          { name: 'Automation clips in playlist', done: false },
          { name: 'Right-click any knob to create automation clip', done: false },
          { name: 'Curve types (smooth, hold, stairs, pulse)', done: false },
          { name: 'Automation recording (record knob movements)', done: false },
          { name: 'LFO-based automation shapes', done: false },
          { name: 'Envelope follower (audio to automation)', done: false },
          { name: 'Tempo-synced automation', done: false },
        ],
      },
      {
        name: 'Advanced Mixer',
        description: 'Full mixing capabilities',
        features: [
          { name: '10 FX insert slots per track (functional)', done: false },
          { name: 'FX enable/disable toggle per slot', done: false },
          { name: 'FX drag reorder', done: false },
          { name: 'Dry/wet mix per FX slot', done: false },
          { name: 'Send routing (any track to any track)', done: false },
          { name: 'Sidechain routing to plugin inputs', done: false },
          { name: 'Per-track stereo L/R peak meters', done: false },
          { name: 'Phase invert & stereo swap', done: false },
          { name: 'Built-in per-track parametric EQ', done: false },
          { name: 'Automatic plugin delay compensation (PDC)', done: false },
        ],
      },
      {
        name: 'Rendering / Export',
        description: 'Bounce to audio files',
        features: [
          { name: 'WAV export (16/24/32-bit, 44.1-192kHz)', done: false },
          { name: 'MP3 export with bitrate control', done: false },
          { name: 'FLAC lossless export', done: false },
          { name: 'OGG Vorbis export', done: false },
          { name: 'Render range (full song / selection / pattern)', done: false },
          { name: 'Split mixer tracks (render stems)', done: false },
          { name: 'Offline faster-than-realtime render', done: false },
          { name: 'Dithering & resampling quality options', done: false },
        ],
      },
    ],
  },
  phase3: {
    title: 'v0.4 — Sampling & Sound Design',
    subtitle: 'Sample editor, slicer, built-in instruments',
    color: '#FF66AA',
    milestones: [
      {
        name: 'Sample Editor',
        description: 'Built-in waveform editor for audio processing',
        features: [
          { name: 'Waveform display with zoom & scroll', done: false },
          { name: 'Cut / copy / paste audio regions', done: false },
          { name: 'Normalize (peak / RMS)', done: false },
          { name: 'Fade in / out', done: false },
          { name: 'Reverse', done: false },
          { name: 'Time stretch & pitch shift', done: false },
          { name: 'Spectral display', done: false },
          { name: 'Loop point editor', done: false },
          { name: 'Record directly into editor', done: false },
          { name: 'Noise removal', done: false },
          { name: 'Detect tempo / auto-BPM', done: false },
        ],
      },
      {
        name: 'Beat Slicer',
        description: 'Slice audio loops into playable pieces',
        features: [
          { name: 'Auto-slice by transient detection', done: false },
          { name: 'Manual slice markers', done: false },
          { name: 'Map slices to keyboard keys', done: false },
          { name: 'Per-slice ADSR envelope', done: false },
          { name: 'Export slices to piano roll pattern', done: false },
          { name: 'Time-stretch individual slices', done: false },
        ],
      },
      {
        name: 'Channel Rack Enhancements',
        description: 'Full channel rack functionality',
        features: [
          { name: 'Variable step count (1-128)', done: false },
          { name: 'Per-step velocity, pan, pitch graph editor', done: false },
          { name: 'Channel types: sampler, audio clip, automation clip, layer', done: false },
          { name: 'Channel volume/pan/pitch/filter knobs', done: false },
          { name: 'Channel grouping & filtering', done: false },
          { name: 'Drag reorder channels', done: false },
          { name: 'Target mixer track routing', done: false },
          { name: 'Clone channels', done: false },
        ],
      },
      {
        name: 'Built-in Sampler Instrument',
        description: 'Multi-sample instrument with zones',
        features: [
          { name: 'Root note & tuning', done: false },
          { name: 'Sample start/end/loop points', done: false },
          { name: 'ADSR volume envelope', done: false },
          { name: 'ADSR filter envelope', done: false },
          { name: 'LFO modulation', done: false },
          { name: 'Key/velocity zone mapping', done: false },
          { name: 'Layer channel (trigger multiple samples)', done: false },
        ],
      },
    ],
  },
  phase4: {
    title: 'v0.5 — Built-in Effects & Instruments',
    subtitle: 'Ship native DSP plugins, reduce dependency on third-party',
    color: hw.yellow,
    milestones: [
      {
        name: 'Core Effects Suite',
        description: 'Essential mixing effects built into the DAW',
        features: [
          { name: 'Parametric EQ (7-band with analyzer)', done: false },
          { name: 'Compressor / Limiter / Gate', done: false },
          { name: 'Reverb (algorithmic)', done: false },
          { name: 'Delay (tempo-synced, filtered feedback)', done: false },
          { name: 'Chorus / Flanger / Phaser', done: false },
          { name: 'Distortion / Saturation / Soft Clip', done: false },
          { name: 'Stereo widener / Mid-Side processor', done: false },
          { name: 'Convolution reverb', done: false },
          { name: 'Multiband compressor', done: false },
          { name: 'De-esser', done: false },
        ],
      },
      {
        name: 'Core Instruments',
        description: 'Built-in synthesizers',
        features: [
          { name: '3-oscillator subtractive synth', done: false },
          { name: 'FM synth (4-6 operator)', done: false },
          { name: 'Wavetable synth', done: false },
          { name: 'Drum machine / pad sampler', done: false },
          { name: 'Simple mono bass synth', done: false },
          { name: 'Kick drum synth', done: false },
        ],
      },
      {
        name: 'Analysis Tools',
        description: 'Visual feedback for mixing and mastering',
        features: [
          { name: 'Spectrum analyzer', done: false },
          { name: 'Oscilloscope', done: false },
          { name: 'Loudness meter (LUFS)', done: false },
          { name: 'Stereo correlation meter', done: false },
          { name: 'Spectrogram (waterfall)', done: false },
        ],
      },
    ],
  },
  phase5: {
    title: 'v1.0 — Production Ready',
    subtitle: 'Polish, performance, and professional workflow',
    color: hw.red,
    milestones: [
      {
        name: 'Advanced Workflow',
        description: 'Power user features',
        features: [
          { name: 'Full undo/redo history with browser', done: false },
          { name: 'Customizable keyboard shortcuts', done: false },
          { name: 'Detachable panels (multi-monitor)', done: false },
          { name: 'Patcher — modular plugin routing', done: false },
          { name: 'Groove/swing templates', done: false },
          { name: 'MIDI import/export (.mid)', done: false },
          { name: 'Plugin preset browser', done: false },
          { name: 'Freeze tracks (offline render to save CPU)', done: false },
          { name: 'Clip grouping and crossfades', done: false },
          { name: 'Time signature changes', done: false },
          { name: 'Tempo automation', done: false },
        ],
      },
      {
        name: 'Performance & Stability',
        description: 'Rock-solid for professional use',
        features: [
          { name: 'Plugin sandboxing (crash isolation)', done: false },
          { name: 'Multi-threaded audio graph', done: false },
          { name: 'Memory-mapped audio file streaming', done: false },
          { name: 'GPU-accelerated waveform rendering', done: false },
          { name: 'ASIO support (Windows)', done: false },
          { name: 'CoreAudio optimizations (macOS)', done: false },
          { name: 'JACK support (Linux)', done: false },
          { name: 'Stress test: 100+ tracks, 50+ plugins', done: false },
        ],
      },
      {
        name: 'Polish',
        description: 'Professional look and feel',
        features: [
          { name: 'Theme engine (custom colors, backgrounds)', done: false },
          { name: 'UI scaling for HiDPI displays', done: false },
          { name: 'Context menus everywhere', done: false },
          { name: 'Tooltips & hint bar', done: false },
          { name: 'Onboarding / welcome wizard', done: false },
          { name: 'Sample browser with preview', done: false },
          { name: 'Plugin thumbnails / screenshots', done: false },
        ],
      },
    ],
  },
  stretch: {
    title: 'Beyond v1.0 — Stretch Goals',
    subtitle: 'Differentiators and long-term vision',
    color: hw.textMuted,
    milestones: [
      {
        name: 'Multiplayer (Hardwave Collabs)',
        description: 'Real-time collaboration between two producers',
        features: [
          { name: 'WebSocket relay server for live sync', done: false },
          { name: 'Room creation with invite codes', done: false },
          { name: 'Live mixer/transport state sync', done: false },
          { name: 'In-DAW chat', done: false },
          { name: 'Presence indicators (cursors, active windows)', done: false },
          { name: 'Conflict resolution with CRDT', done: false },
        ],
      },
      {
        name: 'Marketplace',
        description: 'Buy and sell presets, samples, and plugins',
        features: [
          { name: 'In-app preset store', done: false },
          { name: 'Sample pack browser & purchase', done: false },
          { name: 'Plugin marketplace', done: false },
          { name: 'Creator tools for sellers', done: false },
        ],
      },
      {
        name: 'FL Studio Project Import',
        description: 'Best-effort .flp file parsing (no guarantee of full fidelity)',
        features: [
          { name: 'Parse .flp binary format (reverse-engineered)', done: false, detail: 'Proprietary format — import only, no export. Partial fidelity.' },
          { name: 'Import mixer layout & routing', done: false },
          { name: 'Import piano roll notes', done: false },
          { name: 'Import playlist arrangement', done: false },
          { name: 'Import automation (best effort)', done: false },
          { name: 'Map FL native plugins to Hardwave equivalents', done: false },
        ],
      },
      {
        name: 'Open API & Scripting',
        description: 'Extensible DAW with scripting support',
        features: [
          { name: 'Lua scripting engine for macros', done: false },
          { name: 'REST/WebSocket API for external control', done: false },
          { name: 'MIDI controller scripting (Python)', done: false },
          { name: 'Plugin API for native Hardwave plugins', done: false },
        ],
      },
      {
        name: 'AI-Powered Features',
        description: 'Machine learning tools for production',
        features: [
          { name: 'AI stem separation', done: false },
          { name: 'AI mastering assistant', done: false },
          { name: 'Smart auto-EQ', done: false },
          { name: 'Melody/chord suggestion', done: false },
          { name: 'Auto-categorize samples', done: false },
        ],
      },
    ],
  },
}

const PHASE_ORDER: Phase[] = ['current', 'phase1', 'phase2', 'phase3', 'phase4', 'phase5', 'stretch']

export function Roadmap() {
  const [expandedPhase, setExpandedPhase] = useState<Phase>('current')
  const [expandedMilestones, setExpandedMilestones] = useState<Set<string>>(new Set())

  const toggleMilestone = (key: string) => {
    setExpandedMilestones(prev => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }

  const totalFeatures = Object.values(ROADMAP).flatMap(p => p.milestones.flatMap(m => m.features)).length
  const doneFeatures = Object.values(ROADMAP).flatMap(p => p.milestones.flatMap(m => m.features.filter(f => f.done))).length
  const overallPercent = Math.round((doneFeatures / totalFeatures) * 100)

  return (
    <div style={{
      height: '100%',
      background: hw.bgPanel,
      display: 'flex',
      flexDirection: 'column',
      color: hw.textSecondary,
    }}>
      {/* Header */}
      <div style={{
        padding: '12px 16px',
        background: hw.bg,
        borderBottom: `1px solid ${hw.border}`,
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <span style={{
            fontSize: 14, fontWeight: 800, color: hw.accent,
            letterSpacing: 1,
          }}>
            HARDWAVE DAW ROADMAP
          </span>
          <span style={{ fontSize: 10, color: hw.textFaint }}>
            {doneFeatures} / {totalFeatures} features
          </span>
          <div style={{ flex: 1 }} />
          <span style={{ fontSize: 11, fontWeight: 700, color: hw.accent }}>
            {overallPercent}%
          </span>
        </div>

        <div style={{
          marginTop: 6, height: 4, background: hw.bgInput, borderRadius: 2,
          overflow: 'hidden',
        }}>
          <div style={{
            height: '100%', width: `${overallPercent}%`,
            background: `linear-gradient(90deg, ${hw.secondary}, ${hw.accent})`,
            borderRadius: 2,
            transition: 'width 300ms',
          }} />
        </div>
      </div>

      {/* Phase timeline */}
      <div style={{
        display: 'flex', padding: '0 2px',
        background: hw.bg, borderBottom: `1px solid ${hw.border}`,
      }}>
        {PHASE_ORDER.map(phase => {
          const data = ROADMAP[phase]
          const features = data.milestones.flatMap(m => m.features)
          const done = features.filter(f => f.done).length
          const pct = features.length > 0 ? Math.round((done / features.length) * 100) : 0
          const isActive = expandedPhase === phase

          return (
            <div
              key={phase}
              onClick={() => setExpandedPhase(phase)}
              style={{
                flex: 1,
                padding: '6px 4px',
                textAlign: 'center',
                cursor: 'pointer',
                borderBottom: isActive ? `2px solid ${data.color}` : '2px solid transparent',
                background: isActive ? 'rgba(255,255,255,0.02)' : 'transparent',
              }}
            >
              <div style={{
                fontSize: 8, fontWeight: 700, color: isActive ? data.color : hw.textFaint,
                letterSpacing: 0.5, textTransform: 'uppercase',
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>
                {data.title.split('—')[0].trim()}
              </div>
              <div style={{ fontSize: 7, color: hw.textFaint, marginTop: 1 }}>
                {pct}%
              </div>
            </div>
          )
        })}
      </div>

      {/* Phase content */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '8px 12px' }}>
        {(() => {
          const phase = expandedPhase
          const data = ROADMAP[phase]
          const features = data.milestones.flatMap(m => m.features)
          const done = features.filter(f => f.done).length
          const pct = features.length > 0 ? Math.round((done / features.length) * 100) : 0

          return (
            <div>
              <div style={{ marginBottom: 12 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <div style={{
                    width: 10, height: 10, borderRadius: '50%',
                    background: data.color,
                    boxShadow: `0 0 8px ${data.color}44`,
                  }} />
                  <span style={{ fontSize: 13, fontWeight: 700, color: data.color }}>
                    {data.title}
                  </span>
                </div>
                <div style={{ fontSize: 10, color: hw.textFaint, marginTop: 2, marginLeft: 18 }}>
                  {data.subtitle}
                </div>

                <div style={{ marginTop: 6, marginLeft: 18, display: 'flex', alignItems: 'center', gap: 8 }}>
                  <div style={{
                    flex: 1, maxWidth: 200, height: 3, background: hw.bgInput,
                    borderRadius: 2, overflow: 'hidden',
                  }}>
                    <div style={{
                      height: '100%', width: `${pct}%`,
                      background: data.color, borderRadius: 2,
                    }} />
                  </div>
                  <span style={{ fontSize: 9, color: hw.textFaint }}>
                    {done}/{features.length}
                  </span>
                </div>
              </div>

              {data.milestones.map((milestone, mIdx) => {
                const mKey = `${phase}-${mIdx}`
                const isExpanded = expandedMilestones.has(mKey)
                const mDone = milestone.features.filter(f => f.done).length
                const mTotal = milestone.features.length
                const mPct = mTotal > 0 ? Math.round((mDone / mTotal) * 100) : 0

                return (
                  <div key={mKey} style={{
                    marginBottom: 4,
                    background: hw.bgSurface,
                    borderRadius: hw.radius.md,
                    border: `1px solid ${hw.border}`,
                    overflow: 'hidden',
                  }}>
                    <div
                      onClick={() => toggleMilestone(mKey)}
                      style={{
                        padding: '8px 10px',
                        cursor: 'pointer',
                        display: 'flex',
                        alignItems: 'center',
                        gap: 8,
                      }}
                    >
                      <span style={{
                        fontSize: 9, color: hw.textFaint, fontFamily: 'monospace',
                        transform: isExpanded ? 'rotate(90deg)' : 'none',
                        transition: 'transform 150ms',
                        display: 'inline-block',
                      }}>
                        {'>'}
                      </span>

                      <div style={{ flex: 1 }}>
                        <div style={{ fontSize: 11, fontWeight: 600, color: hw.textPrimary }}>
                          {milestone.name}
                        </div>
                        <div style={{ fontSize: 9, color: hw.textFaint, marginTop: 1 }}>
                          {milestone.description}
                        </div>
                      </div>

                      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <div style={{
                          width: 60, height: 3, background: hw.bgInput,
                          borderRadius: 2, overflow: 'hidden',
                        }}>
                          <div style={{
                            height: '100%', width: `${mPct}%`,
                            background: mPct === 100 ? hw.green : data.color,
                            borderRadius: 2,
                          }} />
                        </div>
                        <span style={{
                          fontSize: 9, color: mPct === 100 ? hw.green : hw.textFaint,
                          fontWeight: mPct === 100 ? 700 : 400,
                          minWidth: 30, textAlign: 'right',
                        }}>
                          {mDone}/{mTotal}
                        </span>
                      </div>
                    </div>

                    {isExpanded && (
                      <div style={{
                        borderTop: `1px solid ${hw.border}`,
                        padding: '4px 0',
                      }}>
                        {milestone.features.map((feature, fIdx) => (
                          <div key={fIdx} style={{
                            padding: '4px 10px 4px 28px',
                            display: 'flex',
                            alignItems: 'flex-start',
                            gap: 8,
                          }}>
                            <div style={{
                              width: 12, height: 12, borderRadius: 3,
                              border: feature.done ? 'none' : `1px solid ${hw.textFaint}`,
                              background: feature.done ? hw.green : 'transparent',
                              display: 'flex', alignItems: 'center', justifyContent: 'center',
                              flexShrink: 0, marginTop: 1,
                            }}>
                              {feature.done && (
                                <span style={{ fontSize: 8, color: hw.bg, fontWeight: 900 }}>
                                  {'\u2713'}
                                </span>
                              )}
                            </div>

                            <div style={{ flex: 1 }}>
                              <div style={{
                                fontSize: 10,
                                color: feature.done ? hw.textFaint : hw.textSecondary,
                                textDecoration: feature.done ? 'line-through' : 'none',
                              }}>
                                {feature.name}
                              </div>
                              {feature.detail && (
                                <div style={{ fontSize: 8, color: hw.textFaint, marginTop: 1 }}>
                                  {feature.detail}
                                </div>
                              )}
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          )
        })()}
      </div>

      {/* Footer */}
      <div style={{
        padding: '8px 12px',
        background: hw.bg,
        borderTop: `1px solid ${hw.border}`,
        display: 'flex',
        gap: 12,
        flexWrap: 'wrap',
      }}>
        {[
          { version: 'v0.1', label: 'Foundation', color: hw.accent, status: 'Released' },
          { version: 'v0.2', label: 'Core Editing', color: '#4488FF', status: 'Next' },
          { version: 'v0.3', label: 'Mix & Automate', color: hw.green, status: 'Planned' },
          { version: 'v0.4', label: 'Sound Design', color: '#FF66AA', status: 'Planned' },
          { version: 'v0.5', label: 'Effects & Synths', color: hw.yellow, status: 'Planned' },
          { version: 'v1.0', label: 'Production Ready', color: hw.red, status: 'Planned' },
        ].map(r => (
          <div key={r.version} style={{
            display: 'flex', alignItems: 'center', gap: 4,
          }}>
            <div style={{
              width: 6, height: 6, borderRadius: '50%',
              background: r.status === 'Released' ? r.color : hw.bgInput,
              border: r.status === 'Next' ? `1px solid ${r.color}` : 'none',
              boxShadow: r.status === 'Released' ? `0 0 6px ${r.color}44` : 'none',
            }} />
            <span style={{
              fontSize: 8, fontWeight: 700, color: r.status === 'Released' ? r.color : hw.textFaint,
            }}>
              {r.version}
            </span>
            <span style={{ fontSize: 8, color: hw.textFaint }}>
              {r.label}
            </span>
          </div>
        ))}
      </div>
    </div>
  )
}
