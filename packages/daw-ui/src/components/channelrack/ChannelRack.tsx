import { useState, useEffect, useRef } from 'react'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'
import { usePatternStore, STEPS_PER_PATTERN, PATTERN_COLORS, STEP_GRAPH_RANGES, STEP_GRAPH_DEFAULTS, type StepGraphKind } from '../../stores/patternStore'
import { DetachButton } from '../FloatingWindow'

const STEPS = STEPS_PER_PATTERN
const DEFAULT_VEL = 0.85

const CHANNEL_COLORS = [
  '#DC2626', '#EF4444', '#F59E0B', '#EAB308', '#10B981', '#06B6D4',
  '#3B82F6', '#8B5CF6', '#A855F7', '#EC4899', '#F43F5E', '#64748B',
]

export function ChannelRack() {
  const { tracks, selectedTrackId, selectTrack, toggleMute, renameTrack, setTrackColor, removeTrack, reorderTrack, addMidiTrack, fetchTracks, setVolume, setPan } = useTrackStore()
  const channels = tracks.filter(t => t.kind !== 'Master')
  const [renamingId, setRenamingId] = useState<string | null>(null)
  const [renameValue, setRenameValue] = useState('')
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; trackId: string } | null>(null)
  const [dragSource, setDragSource] = useState<string | null>(null)
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null)
  const renameInputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (renamingId && renameInputRef.current) {
      renameInputRef.current.focus()
      renameInputRef.current.select()
    }
  }, [renamingId])

  useEffect(() => {
    if (!ctxMenu) return
    const close = () => setCtxMenu(null)
    window.addEventListener('click', close)
    window.addEventListener('contextmenu', close)
    return () => {
      window.removeEventListener('click', close)
      window.removeEventListener('contextmenu', close)
    }
  }, [ctxMenu])

  const startRename = (id: string, currentName: string) => {
    setRenamingId(id)
    setRenameValue(currentName)
  }

  const commitRename = async () => {
    const id = renamingId
    const val = renameValue
    setRenamingId(null)
    if (id && val.trim()) {
      await renameTrack(id, val)
    }
  }

  const cancelRename = () => setRenamingId(null)
  const activePattern = usePatternStore(s => s.patterns.find(p => p.id === s.activeId) || s.patterns[0])
  const patternCount = usePatternStore(s => s.patterns.length)
  const patternIndex = usePatternStore(s => s.patterns.findIndex(p => p.id === s.activeId))
  const getEffectiveLength = usePatternStore(s => s.getEffectiveLength)
  const activePatternLength = getEffectiveLength(activePattern.id)
  const setStep = usePatternStore(s => s.setStep)
  const clearChannel = usePatternStore(s => s.clearChannel)
  const addPattern = usePatternStore(s => s.addPattern)
  const clonePattern = usePatternStore(s => s.clonePattern)
  const deletePattern = usePatternStore(s => s.deletePattern)
  const [swing] = useState(0)
  const [graphEditor, setGraphEditor] = useState(false)
  const [graphMode, setGraphMode] = useState<StepGraphKind>('velocity')
  const setPanStep = usePatternStore(s => s.setPanStep)
  const setPitchStep = usePatternStore(s => s.setPitchStep)
  const setFilterStep = usePatternStore(s => s.setFilterStep)
  const setGateStep = usePatternStore(s => s.setGateStep)
  const getStepGraphValues = usePatternStore(s => s.getStepGraphValues)

  const setGraphStep = (channelId: string, i: number, value: number) => {
    switch (graphMode) {
      case 'velocity': setStep(channelId, i, value); break
      case 'pan': setPanStep(channelId, i, value); break
      case 'pitch': setPitchStep(channelId, i, value); break
      case 'filter': setFilterStep(channelId, i, value); break
      case 'gate': setGateStep(channelId, i, value); break
    }
  }

  const getSteps = (id: string): number[] =>
    activePattern.steps[id] || new Array(STEPS).fill(0)
  const dbToNorm = (db: number) => Math.max(0, Math.min(1, (db + 60) / 72))
  const normToDb = (v: number) => Math.max(-60, Math.min(12, v * 72 - 60))
  const panToNorm = (pan: number) => Math.max(0, Math.min(1, (pan + 1) / 2))
  const normToPan = (v: number) => Math.max(-1, Math.min(1, v * 2 - 1))

  const toggleStep = (id: string, i: number) => {
    const cur = getSteps(id)
    setStep(id, i, cur[i] > 0 ? 0 : DEFAULT_VEL)
  }

  // Vertical drag on a step sets velocity 0..1.
  const startVelocityDrag = (id: string, i: number, e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    const startY = e.clientY
    const startV = getSteps(id)[i] || DEFAULT_VEL
    const move = (ev: MouseEvent) => {
      const dy = startY - ev.clientY
      const v = Math.max(0.05, Math.min(1, startV + dy / 80))
      setStep(id, i, v)
    }
    const up = () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
    }
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
  }

  return (
    <div style={{ height: '100%', background: 'rgba(255,255,255,0.02)', backdropFilter: hw.blur.sm, display: 'flex', flexDirection: 'column' }}>
      {/* Top toolbar */}
      <div style={{
        height: 26, background: 'rgba(255,255,255,0.01)',
        borderBottom: `1px solid ${hw.border}`,
        display: 'flex', alignItems: 'center', padding: '0 4px', gap: 2,
      }}>
        <button style={topBtn} title="Channel options">
          <svg width="8" height="8" viewBox="0 0 8 8"><path d="M1 2h6M1 4h6M1 6h6" stroke={hw.textMuted} strokeWidth="1"/></svg>
        </button>

        <div style={{ display: 'flex', gap: 1 }}>
          {['All', 'Audio', 'MIDI'].map(g => (
            <button key={g} style={{
              ...topBtn, width: 'auto', padding: '0 6px',
              fontSize: 9, color: g === 'All' ? hw.textBright : hw.textMuted,
              background: g === 'All' ? hw.accentDim : 'transparent',
              border: `1px solid ${g === 'All' ? hw.accentGlow : 'transparent'}`,
            }}>
              {g}
            </button>
          ))}
        </div>

        <TbSep />

        <div style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
          <span style={{ fontSize: 8, color: hw.textFaint }}>SWG</span>
          <MiniKnob value={swing} color={hw.textMuted} size={14} />
        </div>

        <TbSep />

        <div style={{ display: 'flex', alignItems: 'center', gap: 2 }}>
          <span style={{ fontSize: 8, color: hw.textFaint }}>Steps</span>
          <span style={{
            fontSize: 10, color: hw.textPrimary, fontWeight: 700,
            fontFamily: "'Consolas', monospace",
            background: 'rgba(255,255,255,0.04)', padding: '1px 5px', borderRadius: hw.radius.sm,
            border: `1px solid ${hw.borderDark}`,
          }}>
            {STEPS}
          </span>
        </div>

        <div style={{ flex: 1 }} />

        <button
          onClick={() => setGraphEditor(v => !v)}
          style={{
            ...topBtn, width: 'auto', padding: '0 6px', fontSize: 9,
            color: graphEditor ? hw.textBright : hw.textFaint,
            background: graphEditor ? hw.accentDim : 'transparent',
          }}
        >
          Graph
        </button>

        <PatternSwitcher />
        <button
          onClick={addPattern}
          title="New pattern"
          style={{ ...topBtn, width: 'auto', padding: '0 6px', fontSize: 9, color: hw.textMuted }}
        >
          +
        </button>
        <button
          onClick={clonePattern}
          title="Clone pattern"
          style={{ ...topBtn, width: 'auto', padding: '0 6px', fontSize: 9, color: hw.textMuted }}
        >
          ⎘
        </button>
        <button
          onClick={deletePattern}
          disabled={patternCount <= 1}
          title={patternCount <= 1 ? 'At least one pattern required' : 'Delete pattern'}
          style={{
            ...topBtn, width: 'auto', padding: '0 6px', fontSize: 9,
            color: patternCount <= 1 ? hw.textFaint : hw.textMuted,
            opacity: patternCount <= 1 ? 0.5 : 1,
          }}
        >
          ×
        </button>
        <span style={{
          fontSize: 9, color: hw.textFaint, fontFamily: "'Consolas', monospace",
          marginLeft: 4,
        }}>
          {patternIndex + 1}/{patternCount}
        </span>
        <DetachButton panelId="channelRack" />
      </div>

      {/* Channel rows */}
      <div style={{ flex: 1, overflowY: 'auto' }}>
        {channels.map((ch, ci) => {
          const selected = selectedTrackId === ch.id
          const vol = dbToNorm(ch.volume_db)
          const pan = panToNorm(ch.pan)
          return (
            <div
              key={ch.id}
              draggable
              onDragStart={(e) => {
                setDragSource(ch.id)
                e.dataTransfer.effectAllowed = 'move'
                e.dataTransfer.setData('text/plain', ch.id)
              }}
              onDragOver={(e) => {
                if (!dragSource || dragSource === ch.id) return
                e.preventDefault()
                e.dataTransfer.dropEffect = 'move'
                const rect = (e.currentTarget as HTMLDivElement).getBoundingClientRect()
                const above = e.clientY < rect.top + rect.height / 2
                setDragOverIndex(above ? ci : ci + 1)
              }}
              onDrop={(e) => {
                e.preventDefault()
                if (!dragSource) return
                const srcIdx = tracks.findIndex(t => t.id === dragSource)
                let target = dragOverIndex ?? ci
                if (srcIdx !== -1 && srcIdx < target) target -= 1
                reorderTrack(dragSource, Math.max(0, target))
                setDragSource(null)
                setDragOverIndex(null)
              }}
              onDragEnd={() => { setDragSource(null); setDragOverIndex(null) }}
              style={{
                height: 30, display: 'flex', alignItems: 'stretch',
                borderBottom: `1px solid ${hw.border}`,
                borderTop: dragOverIndex === ci ? `2px solid ${hw.accent}` : '2px solid transparent',
                opacity: dragSource === ch.id ? 0.4 : 1,
                background: selected ? hw.selectionDim : (ci % 2 === 1 ? 'transparent' : 'rgba(255,255,255,0.015)'),
              }}
            >
              {/* 1. LED */}
              <div
                onClick={() => toggleMute(ch.id)}
                style={{
                  width: 20, display: 'flex', alignItems: 'center', justifyContent: 'center',
                  cursor: 'pointer',
                }}
              >
                <div style={{
                  width: 8, height: 8, borderRadius: '50%',
                  background: ch.muted ? 'rgba(255,255,255,0.06)' : hw.green,
                  boxShadow: ch.muted ? 'none' : `0 0 6px ${hw.greenDim}`,
                }} />
              </div>

              {/* 2. Pan knob */}
              <div style={{ width: 22, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <MiniKnob
                  value={pan}
                  color={hw.yellow}
                  size={14}
                  onChange={(v) => setPan(ch.id, normToPan(v))}
                  onReset={() => setPan(ch.id, 0)}
                  title={`Pan: ${ch.pan === 0 ? 'C' : (ch.pan > 0 ? `R${Math.round(ch.pan * 100)}` : `L${Math.round(-ch.pan * 100)}`)}`}
                />
              </div>

              {/* 3. Volume knob */}
              <div style={{ width: 22, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <MiniKnob
                  value={vol}
                  color={hw.green}
                  size={14}
                  onChange={(v) => setVolume(ch.id, normToDb(v))}
                  onReset={() => setVolume(ch.id, 0)}
                  title={`Volume: ${ch.volume_db.toFixed(1)} dB`}
                />
              </div>

              {/* 4. Mixer track # */}
              <div style={{
                width: 20, display: 'flex', alignItems: 'center', justifyContent: 'center',
                fontSize: 9, color: hw.textFaint, fontFamily: "'Consolas', monospace",
                borderRight: `1px solid ${hw.border}`,
              }}>
                {ci + 1}
              </div>

              {/* 5. Channel name */}
              <div
                onClick={() => selectTrack(ch.id)}
                onDoubleClick={(e) => { e.stopPropagation(); startRename(ch.id, ch.name) }}
                onContextMenu={(e) => {
                  e.preventDefault()
                  e.stopPropagation()
                  setCtxMenu({ x: e.clientX, y: e.clientY, trackId: ch.id })
                }}
                style={{
                  width: 110, minWidth: 110, display: 'flex', alignItems: 'center',
                  padding: '0 6px', cursor: 'default',
                  background: selected ? hw.accentDim : 'rgba(255,255,255,0.03)',
                  borderRight: `1px solid ${hw.border}`,
                  gap: 4,
                }}
              >
                <div style={{
                  width: 3, height: 18, borderRadius: 1,
                  background: ch.color, flexShrink: 0,
                }} />
                {renamingId === ch.id ? (
                  <input
                    ref={renameInputRef}
                    value={renameValue}
                    onChange={(e) => setRenameValue(e.target.value)}
                    onBlur={commitRename}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') { e.preventDefault(); commitRename() }
                      if (e.key === 'Escape') { e.preventDefault(); cancelRename() }
                    }}
                    style={{
                      flex: 1, minWidth: 0, fontSize: 10,
                      background: hw.bg, color: hw.textBright,
                      border: `1px solid ${hw.accent}`, borderRadius: hw.radius.sm,
                      padding: '0 4px', outline: 'none',
                    }}
                  />
                ) : (
                  <span style={{
                    fontSize: 10, color: selected ? hw.textBright : hw.textPrimary,
                    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                    fontWeight: selected ? 600 : 400,
                  }}>
                    {ch.name}
                  </span>
                )}
              </div>

              {/* 6. Channel select dot */}
              <div style={{
                width: 16, display: 'flex', alignItems: 'center', justifyContent: 'center',
                borderRight: `1px solid ${hw.border}`,
              }}>
                <div style={{
                  width: 6, height: 6, borderRadius: '50%',
                  background: selected ? hw.accent : 'transparent',
                  border: `1px solid ${selected ? hw.accent : hw.textFaint}`,
                }} />
              </div>

              {/* 7. Step sequencer — velocity-aware buttons */}
              <div
                style={{ flex: 1, display: 'flex', alignItems: 'center', padding: '0 4px', gap: 1, overflow: 'hidden' }}
                onContextMenu={(e) => { e.preventDefault(); clearChannel(ch.id) }}
              >
                {Array.from({ length: STEPS }, (_, i) => {
                  const vel = getSteps(ch.id)[i] || 0
                  const active = vel > 0
                  const groupIdx = Math.floor(i / 4)
                  const isOddGroup = groupIdx % 2 === 1
                  const inRange = i < activePatternLength
                  return (
                    <button
                      key={i}
                      onClick={() => toggleStep(ch.id, i)}
                      onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); setStep(ch.id, i, 0) }}
                      onMouseDown={(e) => {
                        if (e.button !== 0) return
                        if (!active) return
                        startVelocityDrag(ch.id, i, e)
                      }}
                      title={active ? `Velocity ${Math.round(vel * 127)} (drag up/down)` : 'Click to add step'}
                      style={{
                        flex: 1, maxWidth: 28, height: 22, position: 'relative',
                        background: active
                          ? 'rgba(0,0,0,0.25)'
                          : (isOddGroup ? 'rgba(255,255,255,0.05)' : 'rgba(255,255,255,0.03)'),
                        border: `1px solid ${active ? hw.accentLight : 'rgba(255,255,255,0.06)'}`,
                        borderRadius: hw.radius.sm,
                        boxShadow: active ? `0 0 8px ${hw.accentGlow}` : 'none',
                        marginRight: i % 4 === 3 ? 4 : 0,
                        overflow: 'hidden',
                        transition: 'background 0.05s',
                        padding: 0,
                        opacity: inRange ? 1 : 0.3,
                      }}
                    >
                      {active && (
                        <div style={{
                          position: 'absolute', left: 0, right: 0, bottom: 0,
                          height: `${Math.round(vel * 100)}%`,
                          background: `linear-gradient(180deg, ${hw.accentLight}, ${hw.accent})`,
                          borderRadius: hw.radius.sm,
                          opacity: 0.85 + vel * 0.15,
                          pointerEvents: 'none',
                        }} />
                      )}
                    </button>
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

      {/* Graph editor — per-step value for the selected channel and graph mode */}
      {graphEditor && (() => {
        const target = channels.find(c => c.id === selectedTrackId) || channels[0]
        if (!target) return null
        const values = getStepGraphValues(target.id, graphMode)
        const isBipolar = graphMode === 'pan' || graphMode === 'pitch'
        const range = graphMode === 'velocity'
          ? [0, 1] as [number, number]
          : STEP_GRAPH_RANGES[graphMode]
        const [rMin, rMax] = range
        const fmt = (v: number) => {
          if (graphMode === 'pitch') return `${v >= 0 ? '+' : ''}${v.toFixed(1)} st`
          if (graphMode === 'pan') return v === 0 ? 'C' : (v > 0 ? `R${Math.round(v * 100)}` : `L${Math.round(-v * 100)}`)
          return `${Math.round(v * 100)}%`
        }
        return (
          <div style={{
            height: 96, background: 'rgba(255,255,255,0.02)',
            borderTop: `1px solid ${hw.border}`,
            display: 'flex', flexDirection: 'column', padding: '2px 4px',
            gap: 2,
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 6, height: 16 }}>
              <span style={{ fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
                Graph
              </span>
              <span style={{ fontSize: 9, color: hw.accent, fontWeight: 600 }}>{target.name}</span>
              <div style={{ display: 'flex', gap: 2, marginLeft: 8 }}>
                {(['velocity', 'pan', 'pitch', 'filter', 'gate'] as StepGraphKind[]).map(k => (
                  <button
                    key={k}
                    onClick={() => setGraphMode(k)}
                    style={{
                      padding: '1px 6px', fontSize: 8, textTransform: 'capitalize',
                      background: graphMode === k ? hw.accentDim : 'transparent',
                      border: `1px solid ${graphMode === k ? hw.accent : hw.border}`,
                      borderRadius: hw.radius.sm,
                      color: graphMode === k ? hw.accent : hw.textMuted,
                      cursor: 'pointer',
                    }}
                  >
                    {k}
                  </button>
                ))}
              </div>
            </div>
            <div
              style={{
                flex: 1, display: 'flex', alignItems: 'stretch',
                gap: 1, marginLeft: 202, position: 'relative',
              }}
              onMouseDown={(e) => {
                const wrap = e.currentTarget as HTMLDivElement
                const rect = wrap.getBoundingClientRect()
                const setFromY = (clientX: number, clientY: number) => {
                  const x = clientX - rect.left
                  const y = clientY - rect.top
                  const stepW = rect.width / STEPS
                  const i = Math.max(0, Math.min(STEPS - 1, Math.floor(x / stepW)))
                  const norm = Math.max(0, Math.min(1, 1 - y / rect.height))
                  const value = rMin + norm * (rMax - rMin)
                  setGraphStep(target.id, i, graphMode === 'velocity' && value < 0.02 ? 0 : value)
                }
                setFromY(e.clientX, e.clientY)
                const onMove = (ev: MouseEvent) => setFromY(ev.clientX, ev.clientY)
                const onUp = () => {
                  window.removeEventListener('mousemove', onMove)
                  window.removeEventListener('mouseup', onUp)
                }
                window.addEventListener('mousemove', onMove)
                window.addEventListener('mouseup', onUp)
              }}
            >
              {isBipolar && (
                <div style={{
                  position: 'absolute', left: 0, right: 0, top: '50%',
                  borderTop: `1px dashed ${hw.textFaint}`, pointerEvents: 'none',
                }} />
              )}
              {Array.from({ length: STEPS }, (_, i) => {
                const raw = values[i] ?? (graphMode === 'velocity' ? 0 : STEP_GRAPH_DEFAULTS[graphMode as Exclude<StepGraphKind, 'velocity'>])
                const norm = (raw - rMin) / (rMax - rMin)
                const inRange = i < activePatternLength
                const velStep = graphMode === 'velocity' ? (getSteps(target.id)[i] || 0) : null
                const active = velStep !== null ? velStep > 0 : true
                let barTop = 0
                let barBottom = 0
                if (isBipolar) {
                  if (norm >= 0.5) { barTop = (1 - norm) * 100; barBottom = 50 }
                  else { barTop = 50; barBottom = norm * 100 }
                } else {
                  barTop = (1 - norm) * 100
                  barBottom = 0
                }
                return (
                  <div key={i} style={{
                    flex: 1, maxWidth: 28, position: 'relative',
                    marginRight: i % 4 === 3 ? 4 : 0,
                    opacity: inRange ? (active ? 1 : 0.4) : 0.2,
                    cursor: 'ns-resize',
                  }}
                    title={`Step ${i + 1}: ${fmt(raw)}`}
                  >
                    <div style={{
                      position: 'absolute', left: 0, right: 0,
                      top: `${barTop}%`, bottom: `${barBottom}%`,
                      background: hw.accentDim,
                      border: `1px solid ${hw.accentGlow}`,
                      borderRadius: 2,
                    }} />
                  </div>
                )
              })}
            </div>
          </div>
        )
      })()}

      {/* Bottom bar */}
      <div style={{
        height: 22, background: 'rgba(255,255,255,0.01)',
        borderTop: `1px solid ${hw.border}`,
        display: 'flex', alignItems: 'center', padding: '0 6px', gap: 4,
      }}>
        <span style={{ fontSize: 8, color: hw.textFaint }}>{channels.length} channels</span>
        <div style={{ flex: 1 }} />
        <button style={{
          ...topBtn, width: 'auto', padding: '0 6px', fontSize: 9,
          color: hw.textSecondary, gap: 2, display: 'flex', alignItems: 'center',
        }}>
          <svg width="8" height="8" viewBox="0 0 8 8">
            <line x1="4" y1="1" x2="4" y2="7" stroke="currentColor" strokeWidth="1.2"/>
            <line x1="1" y1="4" x2="7" y2="4" stroke="currentColor" strokeWidth="1.2"/>
          </svg>
          Add
        </button>
      </div>

      {ctxMenu && (
        <div
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
          style={{
            position: 'fixed', left: ctxMenu.x, top: ctxMenu.y, zIndex: 10000,
            minWidth: 168,
            background: 'rgba(12,12,18,0.96)',
            border: `1px solid ${hw.borderLight}`,
            borderRadius: hw.radius.md,
            boxShadow: '0 8px 32px rgba(0,0,0,0.55)',
            padding: 4,
            backdropFilter: hw.blur.md,
          }}
        >
          <MenuItem
            label="Rename"
            shortcut="F2"
            onClick={() => {
              const id = ctxMenu.trackId
              const t = channels.find(c => c.id === id)
              setCtxMenu(null)
              if (t) startRename(id, t.name)
            }}
          />
          <MenuItem
            label="Clone"
            shortcut=""
            onClick={async () => {
              const id = ctxMenu.trackId
              const src = channels.find(c => c.id === id)
              setCtxMenu(null)
              if (!src) return
              const before = useTrackStore.getState().tracks.map(t => t.id)
              await addMidiTrack(`${src.name} (copy)`)
              await fetchTracks()
              const after = useTrackStore.getState().tracks
              const newTrack = after.find(t => !before.includes(t.id))
              if (!newTrack) return
              if (src.color) await setTrackColor(newTrack.id, src.color)
              const store = usePatternStore.getState()
              const updatedPatterns = store.patterns.map(p => {
                const srcSteps = p.steps[src.id]
                if (!srcSteps) return p
                return { ...p, steps: { ...p.steps, [newTrack.id]: [...srcSteps] } }
              })
              usePatternStore.setState({ patterns: updatedPatterns })
            }}
          />
          <MenuSep />
          <div style={{ padding: '4px 8px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
            Color
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(6, 1fr)', gap: 2, padding: '2px 6px 4px' }}>
            {CHANNEL_COLORS.map(c => (
              <button
                key={c}
                title={c}
                onClick={async () => {
                  const id = ctxMenu.trackId
                  setCtxMenu(null)
                  await setTrackColor(id, c)
                }}
                style={{
                  width: 18, height: 18, borderRadius: hw.radius.sm,
                  background: c, border: '1px solid rgba(255,255,255,0.12)',
                  cursor: 'pointer', padding: 0,
                }}
              />
            ))}
          </div>
          <MenuSep />
          <MenuItem
            label="Delete"
            shortcut="Del"
            danger
            onClick={async () => {
              const id = ctxMenu.trackId
              setCtxMenu(null)
              await removeTrack(id)
            }}
          />
        </div>
      )}
    </div>
  )
}

function MenuItem({ label, shortcut, danger, onClick }: {
  label: string
  shortcut?: string
  danger?: boolean
  onClick: () => void
}) {
  const [hover, setHover] = useState(false)
  return (
    <button
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onClick={onClick}
      style={{
        width: '100%', display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        padding: '5px 8px', border: 'none', background: hover ? hw.accentDim : 'transparent',
        color: danger ? hw.red : (hover ? hw.textBright : hw.textPrimary),
        fontSize: 10, cursor: 'pointer', borderRadius: hw.radius.sm, textAlign: 'left',
      }}
    >
      <span>{label}</span>
      {shortcut && (
        <span style={{ fontSize: 9, color: hw.textFaint, marginLeft: 12 }}>{shortcut}</span>
      )}
    </button>
  )
}

function MenuSep() {
  return <div style={{ height: 1, background: hw.border, margin: '2px 0' }} />
}

function MiniKnob({ value, color, size, onChange, onReset, title }: {
  value: number
  color: string
  size: number
  onChange?: (v: number) => void
  onReset?: () => void
  title?: string
}) {
  const angle = -135 + value * 270
  const r = size / 2 - 1
  const cx = size / 2
  const cy = size / 2
  const rad = (angle * Math.PI) / 180
  const endX = cx + (r - 2) * Math.sin(rad)
  const endY = cy - (r - 2) * Math.cos(rad)

  const handleMouseDown = (e: React.MouseEvent) => {
    if (!onChange) return
    e.preventDefault()
    e.stopPropagation()
    const startY = e.clientY
    const startV = value
    const shift = e.shiftKey
    const onMove = (ev: MouseEvent) => {
      const dy = startY - ev.clientY
      const delta = dy / (shift ? 400 : 120)
      onChange(Math.max(0, Math.min(1, startV + delta)))
    }
    const onUp = () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }

  return (
    <svg
      width={size} height={size} viewBox={`0 0 ${size} ${size}`}
      onMouseDown={handleMouseDown}
      onDoubleClick={(e) => { e.stopPropagation(); onReset?.() }}
      style={{ cursor: onChange ? 'ns-resize' : 'default' }}
    >
      <title>{title}</title>
      <circle cx={cx} cy={cy} r={r} fill="rgba(255,255,255,0.03)" stroke="rgba(255,255,255,0.06)" strokeWidth="0.5" />
      <line x1={cx} y1={cy} x2={endX} y2={endY} stroke={color} strokeWidth="1.2" strokeLinecap="round" />
      <circle cx={cx} cy={cy} r="1" fill={color} opacity="0.4" />
    </svg>
  )
}

function TbSep() {
  return <div style={{ width: 1, height: 14, background: hw.border, margin: '0 2px' }} />
}

function PatternSwitcher() {
  const patterns = usePatternStore(s => s.patterns)
  const activeId = usePatternStore(s => s.activeId)
  const setActive = usePatternStore(s => s.setActive)
  const renamePattern = usePatternStore(s => s.renamePattern)
  const setPatternColor = usePatternStore(s => s.setPatternColor)
  const setPatternLength = usePatternStore(s => s.setPatternLength)
  const getEffectiveLength = usePatternStore(s => s.getEffectiveLength)
  const prev = usePatternStore(s => s.prevPattern)
  const next = usePatternStore(s => s.nextPattern)
  const active = patterns.find(p => p.id === activeId) || patterns[0]
  const [renaming, setRenaming] = useState(false)
  const [draft, setDraft] = useState('')
  const [paletteOpen, setPaletteOpen] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)
  const paletteRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!paletteOpen) return
    const onDoc = (e: MouseEvent) => {
      if (paletteRef.current && !paletteRef.current.contains(e.target as Node)) {
        setPaletteOpen(false)
      }
    }
    window.addEventListener('mousedown', onDoc)
    return () => window.removeEventListener('mousedown', onDoc)
  }, [paletteOpen])

  useEffect(() => {
    if (renaming && inputRef.current) {
      inputRef.current.focus()
      inputRef.current.select()
    }
  }, [renaming])

  const commit = () => {
    const v = draft.trim()
    setRenaming(false)
    if (v && v !== active.name) renamePattern(active.id, v)
  }

  const activeColor = active.color || PATTERN_COLORS[0]
  const effectiveLen = getEffectiveLength(active.id)

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 2, position: 'relative' }}>
      <div ref={paletteRef} style={{ position: 'relative' }}>
        <button
          onMouseDown={(e) => { e.preventDefault(); e.stopPropagation(); setPaletteOpen(v => !v) }}
          title="Pattern color"
          style={{
            width: 14, height: 14, padding: 0,
            background: activeColor,
            border: `1px solid ${hw.border}`,
            borderRadius: hw.radius.sm,
            cursor: 'pointer', flexShrink: 0,
          }}
        />
        {paletteOpen && (
          <div style={{
            position: 'absolute', top: 18, left: 0, zIndex: 50,
            display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 3,
            padding: 4,
            background: 'rgba(12,12,18,0.96)',
            border: `1px solid ${hw.border}`,
            borderRadius: hw.radius.sm,
            boxShadow: '0 4px 12px rgba(0,0,0,0.4)',
          }}>
            {PATTERN_COLORS.map(c => (
              <button
                key={c}
                onClick={() => { setPatternColor(active.id, c); setPaletteOpen(false) }}
                style={{
                  width: 16, height: 16, padding: 0,
                  background: c,
                  border: active.color === c ? `1.5px solid ${hw.textBright}` : `1px solid ${hw.border}`,
                  borderRadius: hw.radius.sm,
                  cursor: 'pointer',
                }}
              />
            ))}
          </div>
        )}
      </div>
      <button onClick={prev} title="Previous pattern" style={{
        ...topBtn, width: 16, fontSize: 10, color: hw.textMuted,
      }}>
        ‹
      </button>
      {renaming ? (
        <input
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === 'Enter') { e.preventDefault(); commit() }
            if (e.key === 'Escape') { e.preventDefault(); setRenaming(false) }
          }}
          style={{
            fontSize: 10, color: hw.textBright, fontWeight: 600,
            background: hw.bg,
            border: `1px solid ${hw.accent}`,
            borderRadius: hw.radius.sm,
            padding: '2px 6px', outline: 'none',
            minWidth: 100,
          }}
        />
      ) : (
        <select
          value={active.id}
          onChange={(e) => setActive(e.target.value)}
          onDoubleClick={(e) => {
            e.preventDefault()
            setDraft(active.name)
            setRenaming(true)
          }}
          title="Double-click to rename"
          data-testid="pattern-select"
          style={{
            fontSize: 10, color: hw.textPrimary, fontWeight: 600,
            background: 'rgba(255,255,255,0.04)',
            border: `1px solid ${hw.border}`,
            borderRadius: hw.radius.sm,
            padding: '2px 6px', outline: 'none', appearance: 'none',
            minWidth: 100, cursor: 'pointer',
          }}
        >
          {patterns.map(p => (
            <option key={p.id} value={p.id}>{p.name}</option>
          ))}
        </select>
      )}
      <button onClick={next} title="Next pattern" style={{
        ...topBtn, width: 16, fontSize: 10, color: hw.textMuted,
      }}>
        ›
      </button>
      <input
        type="number"
        min={1}
        max={64}
        value={active.length ?? effectiveLen}
        onChange={(e) => {
          const v = parseInt(e.target.value, 10)
          setPatternLength(active.id, Number.isFinite(v) && v > 0 ? v : undefined)
        }}
        onDoubleClick={() => setPatternLength(active.id, undefined)}
        title={active.length
          ? `Pattern length: ${active.length} steps (double-click to auto)`
          : `Auto length: ${effectiveLen} steps (edit to override)`}
        style={{
          width: 32, fontSize: 9, color: active.length ? hw.accent : hw.textFaint,
          background: 'rgba(255,255,255,0.04)',
          border: `1px solid ${hw.border}`,
          borderRadius: hw.radius.sm,
          padding: '1px 3px', outline: 'none', textAlign: 'center',
          fontVariantNumeric: 'tabular-nums',
        }}
      />
      {[4, 8, 16].map(n => {
        const activeLen = active.length ?? effectiveLen
        const on = activeLen === n
        return (
          <button
            key={n}
            onClick={() => setPatternLength(active.id, n)}
            title={`${n} steps`}
            style={{
              minWidth: 18, height: 16, fontSize: 9, fontWeight: 600, padding: '0 3px',
              color: on ? hw.accent : hw.textFaint,
              background: on ? hw.accentDim : 'rgba(255,255,255,0.04)',
              border: `1px solid ${on ? hw.accentGlow : hw.border}`,
              borderRadius: hw.radius.sm, cursor: 'pointer',
            }}
          >
            {n}
          </button>
        )
      })}
    </div>
  )
}

const topBtn: React.CSSProperties = {
  width: 22, height: 18,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  background: 'transparent', border: '1px solid transparent',
  borderRadius: 6, cursor: 'pointer',
}
