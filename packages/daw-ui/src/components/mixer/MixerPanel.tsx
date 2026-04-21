import { hw } from '../../theme'
import { useTrackStore, type InsertInfo } from '../../stores/trackStore'
import { usePluginStore } from '../../stores/pluginStore'
import { useMeterStore, DEFAULT_TRACK_METER } from '../../stores/meterStore'
import { PATTERN_COLORS } from '../../stores/patternStore'
import { DetachButton } from '../FloatingWindow'
import { ParameterContextMenu } from '../ParameterContextMenu'
import { useEffect, useState, useCallback, useRef } from 'react'

export function MixerPanel() {
  const { tracks, setVolume, setPan, toggleMute, toggleSolo, toggleSoloSafe, toggleArm, renameTrack, setTrackColor } = useTrackStore()
  const { master, tracks: trackMeters, startListening } = useMeterStore()
  useEffect(() => { startListening() }, [])
  const [clipResetNonce, setClipResetNonce] = useState(0)

  const allTracks = tracks.filter(t => t.kind !== 'Master')
  const masterTrack = tracks.find(t => t.kind === 'Master')

  return (
    <div style={{ height: '100%', background: 'rgba(255,255,255,0.02)', backdropFilter: hw.blur.sm, display: 'flex', flexDirection: 'column' }}>
      <div style={{
        height: 22, background: 'rgba(255,255,255,0.01)', borderBottom: `1px solid ${hw.border}`,
        display: 'flex', alignItems: 'center', padding: '0 4px 0 8px', gap: 4,
      }}>
        <span style={{ flex: 1, fontSize: 10, fontWeight: 600, color: hw.textMuted }}>Mixer</span>
        <button
          data-testid="mixer-reset-meters"
          onClick={() => setClipResetNonce(n => n + 1)}
          title="Reset all clip indicators"
          style={{
            height: 16, padding: '0 6px', fontSize: 8, fontWeight: 600,
            color: hw.textMuted, background: 'rgba(255,255,255,0.04)',
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, cursor: 'pointer',
          }}
        >RESET</button>
        <DetachButton panelId="mixer" />
      </div>

      <div style={{
        flex: 1, display: 'flex',
        overflowX: 'auto', overflowY: 'hidden',
        padding: 4, gap: 2,
      }}>
        {/* dB scale ruler (once per mixer, to the left of the first strip) */}
        {allTracks.length > 0 && <DbScale />}

        {allTracks.map((track, idx) => {
          const meter = trackMeters[track.id] ?? DEFAULT_TRACK_METER
          return (
            <Strip key={track.id}
              trackId={track.id}
              inserts={track.inserts ?? []}
              name={track.name} color={track.color} number={idx}
              volumeDb={track.volume_db} pan={track.pan}
              muted={track.muted} soloed={track.soloed}
              soloSafe={track.solo_safe}
              armed={track.armed}
              peakL={meter.peakL} peakR={meter.peakR} rmsDb={meter.rms}
              clipResetNonce={clipResetNonce}
              onRename={name => renameTrack(track.id, name)}
              onColorChange={color => setTrackColor(track.id, color)}
              onVolume={db => setVolume(track.id, db)}
              onPan={p => setPan(track.id, p)}
              onMute={() => toggleMute(track.id)}
              onSolo={() => toggleSolo(track.id)}
              onArm={() => toggleArm(track.id)}
              onToggleSoloSafe={() => toggleSoloSafe(track.id)}
            />
          )
        })}

        {allTracks.length > 0 && <div style={{ width: 1, background: hw.border, margin: '0 1px', flexShrink: 0 }} />}

        <Strip
          name="Master" color={hw.accent} number={-1}
          volumeDb={masterTrack?.volume_db ?? 0} pan={0}
          muted={masterTrack?.muted ?? false} soloed={false}
          peakL={master.peak_db} peakR={master.peak_db}
          rmsDb={master.rms_db}
          peakHoldDb={master.peak_hold_db}
          clipResetNonce={clipResetNonce}
          isMaster
          onVolume={masterTrack ? db => setVolume(masterTrack.id, db) : undefined}
          onPan={() => {}} onMute={masterTrack ? () => toggleMute(masterTrack.id) : undefined}
          onSolo={() => {}}
        />

        {allTracks.length === 0 && (
          <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', color: hw.textFaint, fontSize: 10 }}>
            Add tracks to see mixer
          </div>
        )}
      </div>
    </div>
  )
}

// Meter dB range used by every strip.
const METER_MIN_DB = -60
const METER_MAX_DB = 6

function dbToPct(db: number) {
  const clamped = Math.max(METER_MIN_DB, Math.min(METER_MAX_DB, db))
  return ((clamped - METER_MIN_DB) / (METER_MAX_DB - METER_MIN_DB)) * 100
}

function DbScale() {
  const marks = [6, 0, -6, -12, -24, -36, -48, -60]
  return (
    <div data-testid="db-scale" style={{
      width: 18, flexShrink: 0, position: 'relative',
      display: 'flex', flexDirection: 'column',
      paddingTop: 22 /* match color bar + header area so ticks align with meter top */,
      paddingBottom: 30 /* dB readout + M/S row */,
    }}>
      {/* We align ticks to the visible meter height. The meter lives inside Strip
          starting after: color bar (2) + header (~22) + FX slots (~42) + clip led (6+gap).
          Easier: render ticks on a relative span that Strip layout also honors. */}
      <div style={{ flex: 1, position: 'relative' }}>
        {marks.map(db => {
          // Meters in Strip occupy the "fader row" area. We map db→pct of that area.
          const pct = 100 - dbToPct(db)
          return (
            <div key={db} data-testid={`db-mark-${db}`} style={{
              position: 'absolute', right: 1, left: 0, top: `${pct}%`,
              transform: 'translateY(-50%)',
              display: 'flex', alignItems: 'center', justifyContent: 'flex-end',
              gap: 2,
            }}>
              <span style={{ fontSize: 7, color: hw.textFaint, fontFamily: "'Consolas', monospace" }}>
                {db > 0 ? `+${db}` : db}
              </span>
              <div style={{ width: 3, height: 1, background: hw.textFaint, opacity: 0.5 }} />
            </div>
          )
        })}
      </div>
    </div>
  )
}

interface StripProps {
  trackId?: string
  inserts?: InsertInfo[]
  name: string; color: string; number: number
  volumeDb: number; pan: number; muted: boolean; soloed: boolean
  soloSafe?: boolean
  armed?: boolean
  peakL: number; peakR: number; rmsDb: number
  peakHoldDb?: number
  clipResetNonce?: number
  isMaster?: boolean
  onRename?: (name: string) => void
  onColorChange?: (color: string) => void
  onVolume?: (db: number) => void; onPan: (p: number) => void
  onMute?: () => void; onSolo: () => void
  onArm?: () => void
  onToggleSoloSafe?: () => void
}

function Strip({ trackId, inserts = [], name, color, number, volumeDb, muted, soloed, soloSafe, armed, peakL, peakR, rmsDb, peakHoldDb, clipResetNonce, isMaster, onRename, onColorChange, onVolume, onPan, onMute, onSolo, onArm, onToggleSoloSafe }: StripProps) {
  const [clipped, setClipped] = useState(false)
  const resetClip = useCallback(() => setClipped(false), [])
  useEffect(() => { if (peakL >= 0 || peakR >= 0) setClipped(true) }, [peakL, peakR])
  useEffect(() => { setClipped(false) }, [clipResetNonce])

  const [editing, setEditing] = useState(false)
  const [draftName, setDraftName] = useState(name)
  useEffect(() => { setDraftName(name) }, [name])
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => { if (editing) inputRef.current?.select() }, [editing])
  const commitName = () => {
    const trimmed = draftName.trim()
    if (trimmed && trimmed !== name) onRename?.(trimmed)
    else setDraftName(name)
    setEditing(false)
  }

  const [colorOpen, setColorOpen] = useState(false)
  useEffect(() => {
    if (!colorOpen) return
    const close = (e: MouseEvent) => {
      const t = e.target as HTMLElement
      if (t.closest('[data-color-popover]')) return
      setColorOpen(false)
    }
    window.addEventListener('click', close)
    return () => window.removeEventListener('click', close)
  }, [colorOpen])

  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number } | null>(null)
  const [faderCtx, setFaderCtx] = useState<{ x: number; y: number } | null>(null)
  useEffect(() => {
    if (!ctxMenu) return
    const close = (e: MouseEvent) => {
      const t = e.target as HTMLElement
      if (t.closest('[data-strip-ctx-menu]')) return
      setCtxMenu(null)
    }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [ctxMenu])

  const meterColor = (db: number) => db > -3 ? hw.red : db > -12 ? hw.yellow : hw.green

  return (
    <div
      data-testid={`mixer-strip-${isMaster ? 'master' : name}`}
      onContextMenu={e => {
        if (isMaster) return
        e.preventDefault()
        setCtxMenu({ x: e.clientX, y: e.clientY })
      }}
      style={{
        width: 62, minWidth: 62,
        display: 'flex', flexDirection: 'column',
        background: 'rgba(255,255,255,0.03)', border: `1px solid ${hw.border}`,
        borderRadius: hw.radius.lg, flexShrink: 0, overflow: 'hidden',
        position: 'relative',
      }}
    >
      {/* Color bar */}
      <div
        onClick={e => { if (!isMaster && onColorChange) { e.stopPropagation(); setColorOpen(v => !v) } }}
        title={!isMaster && onColorChange ? 'Click to change track color' : undefined}
        style={{
          height: 4, background: color, cursor: !isMaster && onColorChange ? 'pointer' : 'default',
          position: 'relative',
        }}
      >
        {colorOpen && !isMaster && (
          <div data-color-popover style={{
            position: 'absolute', top: 6, left: 0, zIndex: 20,
            display: 'grid', gridTemplateColumns: 'repeat(4, 14px)', gap: 2, padding: 4,
            background: '#111', border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
            boxShadow: '0 4px 16px rgba(0,0,0,0.6)',
          }}>
            {PATTERN_COLORS.map(c => (
              <button key={c} onClick={() => { onColorChange?.(c); setColorOpen(false) }}
                style={{
                  width: 14, height: 14, background: c, border: c === color ? '2px solid #fff' : '1px solid rgba(255,255,255,0.1)',
                  borderRadius: 3, padding: 0, cursor: 'pointer',
                }}
                title={c}
              />
            ))}
          </div>
        )}
      </div>

      {/* Header */}
      <div style={{ padding: '3px 3px 2px', textAlign: 'center', borderBottom: `1px solid ${hw.border}` }}>
        <div style={{ fontSize: 8, color: hw.textFaint }}>{isMaster ? 'M' : number}</div>
        {editing && !isMaster ? (
          <input
            ref={inputRef}
            value={draftName}
            onChange={e => setDraftName(e.target.value)}
            onBlur={commitName}
            onKeyDown={e => {
              if (e.key === 'Enter') commitName()
              else if (e.key === 'Escape') { setDraftName(name); setEditing(false) }
            }}
            style={{
              width: '100%', fontSize: 9, fontWeight: 500, textAlign: 'center',
              color: hw.textPrimary, background: 'rgba(255,255,255,0.08)',
              border: `1px solid ${hw.accent}`, borderRadius: 2, padding: '1px 2px',
              outline: 'none', boxSizing: 'border-box',
            }}
          />
        ) : (
          <div
            onDoubleClick={() => { if (!isMaster && onRename) setEditing(true) }}
            title={!isMaster && onRename ? 'Double-click to rename' : undefined}
            style={{
              fontSize: 9, fontWeight: 500, color: hw.textPrimary,
              overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              cursor: !isMaster && onRename ? 'text' : 'default',
            }}
          >{name}</div>
        )}
      </div>

      {/* FX slots */}
      <FxSlots
        trackId={isMaster ? undefined : trackId}
        inserts={inserts}
      />

      {/* Fader + meter */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', padding: 3, gap: 2, minHeight: 70 }}>
        {/* Clip indicator */}
        <div
          onClick={resetClip}
          title={clipped ? 'Click to reset clip indicator' : undefined}
          style={{
            width: '100%', height: 6, borderRadius: hw.radius.sm,
            background: clipped ? hw.red : 'rgba(255,255,255,0.04)',
            border: `1px solid ${clipped ? hw.red : hw.borderDark}`,
            cursor: clipped ? 'pointer' : 'default',
            transition: 'background 0.1s',
            flexShrink: 0,
          }}
        />
        <div style={{ flex: 1, display: 'flex', gap: 2 }}>
        {/* Dual meter with RMS overlay */}
        <div style={{ display: 'flex', gap: 1, width: 10, flexShrink: 0 }}>
          {[
            { db: peakL, label: 'L' },
            { db: peakR, label: 'R' },
          ].map(({ db, label }) => (
            <div key={label} data-testid={`meter-${isMaster ? 'master' : name}-${label}`} style={{
              flex: 1, background: 'rgba(255,255,255,0.03)', position: 'relative',
              border: `1px solid ${hw.borderDark}`, borderRadius: hw.radius.sm,
              overflow: 'hidden',
            }}>
              {/* Peak bar */}
              <div data-testid={`meter-peak-${isMaster ? 'master' : name}-${label}`} data-db={db.toFixed(2)} style={{
                position: 'absolute', bottom: 0, left: 0, right: 0,
                height: `${dbToPct(db)}%`, background: meterColor(db),
                transition: 'height 60ms',
              }} />
              {/* RMS overlay (translucent lighter band) */}
              <div data-testid={`meter-rms-${isMaster ? 'master' : name}-${label}`} data-db={rmsDb.toFixed(2)} style={{
                position: 'absolute', bottom: 0, left: 0, right: 0,
                height: `${dbToPct(rmsDb)}%`,
                background: 'rgba(255,255,255,0.35)',
                mixBlendMode: 'overlay',
                transition: 'height 100ms',
              }} />
              {/* Peak-hold tick (master only — per-track meters don't track hold yet) */}
              {peakHoldDb !== undefined && peakHoldDb > METER_MIN_DB && (
                <div style={{
                  position: 'absolute', left: 0, right: 0,
                  bottom: `${dbToPct(peakHoldDb)}%`, height: 1,
                  background: hw.textPrimary,
                }} />
              )}
            </div>
          ))}
        </div>

        {/* Fader */}
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <input type="range" min={-60} max={12} step={0.1} value={volumeDb}
            onChange={e => onVolume?.(parseFloat(e.target.value))}
            onContextMenu={e => {
              if (!onVolume) return
              e.preventDefault()
              e.stopPropagation()
              setFaderCtx({ x: e.clientX, y: e.clientY })
            }}
            style={{
              writingMode: 'vertical-lr' as any, direction: 'rtl',
              flex: 1, width: 16, appearance: 'none',
              background: 'rgba(255,255,255,0.04)',
              border: `1px solid ${hw.borderDark}`, borderRadius: hw.radius.sm, cursor: 'pointer',
            }}
          />
        </div>
        </div>
      </div>

      {/* dB readout */}
      <div style={{ textAlign: 'center', padding: '2px 0', borderTop: `1px solid ${hw.border}` }}>
        <span style={{ fontSize: 8, color: hw.textPrimary, fontFamily: "'Consolas', monospace" }}>
          {volumeDb > -60 ? `${volumeDb.toFixed(1)}` : '-\u221E'}
        </span>
      </div>

      {/* M/S/R buttons */}
      <div style={{ display: 'flex', gap: 1, padding: '2px 3px 3px', justifyContent: 'center' }}>
        <button onClick={() => onMute?.()} style={{
          ...sB, color: muted ? hw.red : hw.textMuted,
          background: muted ? hw.redDim : 'rgba(255,255,255,0.03)',
        }}>M</button>
        <button
          onClick={onSolo}
          title={soloSafe ? 'Solo (solo-safe on: ignores other solos)' : 'Solo'}
          style={{
            ...sB, color: soloed ? hw.yellow : hw.textMuted,
            background: soloed ? hw.yellowDim : 'rgba(255,255,255,0.03)',
            outline: soloSafe ? `1px solid ${hw.accent}` : undefined,
            outlineOffset: soloSafe ? -2 : undefined,
          }}
        >S</button>
        {!isMaster && onArm && (
          <button
            onClick={onArm}
            title="Arm for recording"
            style={{
              ...sB, color: armed ? hw.red : hw.textMuted,
              background: armed ? hw.redDim : 'rgba(255,255,255,0.03)',
            }}
          >R</button>
        )}
      </div>
      {ctxMenu && !isMaster && (
        <div
          data-strip-ctx-menu
          onMouseDown={e => e.stopPropagation()}
          style={{
            position: 'fixed', left: ctxMenu.x, top: ctxMenu.y, zIndex: 10000,
            minWidth: 180, padding: 4,
            background: 'rgba(12,12,18,0.96)',
            border: `1px solid ${hw.borderLight}`,
            borderRadius: hw.radius.md,
            boxShadow: '0 8px 32px rgba(0,0,0,0.55)',
            backdropFilter: hw.blur.md,
          }}
        >
          <div style={{ padding: '4px 8px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
            {name}
          </div>
          <StripMenuItem label="Rename" shortcut="F2" onClick={() => { setCtxMenu(null); setEditing(true) }} />
          <StripMenuItem label="Reset volume (0 dB)" onClick={() => { setCtxMenu(null); onVolume?.(0) }} />
          <StripMenuItem label="Reset pan (center)" onClick={() => { setCtxMenu(null); onPan(0) }} />
          <StripMenuItem label={muted ? 'Unmute' : 'Mute'} onClick={() => { setCtxMenu(null); onMute?.() }} />
          <StripMenuItem label={soloed ? 'Unsolo' : 'Solo'} onClick={() => { setCtxMenu(null); onSolo() }} />
          {onToggleSoloSafe && (
            <StripMenuItem
              label={soloSafe ? 'Disable solo-safe' : 'Solo-safe'}
              onClick={() => { setCtxMenu(null); onToggleSoloSafe() }}
            />
          )}
          <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
          <div style={{ padding: '4px 8px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
            Color
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(6, 1fr)', gap: 2, padding: '2px 6px 6px' }}>
            {PATTERN_COLORS.map(c => (
              <button key={c} title={c}
                onClick={() => { setCtxMenu(null); onColorChange?.(c) }}
                style={{
                  width: 18, height: 18, borderRadius: hw.radius.sm,
                  background: c, border: c === color ? '2px solid #fff' : '1px solid rgba(255,255,255,0.12)',
                  cursor: 'pointer', padding: 0,
                }}
              />
            ))}
          </div>
        </div>
      )}
      {faderCtx && onVolume && (
        <ParameterContextMenu
          x={faderCtx.x}
          y={faderCtx.y}
          label={isMaster ? 'Master volume' : `${name} volume`}
          value={volumeDb}
          defaultValue={0}
          unit=" dB"
          min={-60}
          max={12}
          decimals={1}
          onSet={v => onVolume(v)}
          onClose={() => setFaderCtx(null)}
        />
      )}
    </div>
  )
}

function StripMenuItem({ label, shortcut, onClick }: {
  label: string; shortcut?: string; onClick: () => void
}) {
  return (
    <button
      onClick={onClick}
      style={{
        width: '100%', display: 'flex', alignItems: 'center',
        padding: '5px 8px', gap: 8, border: 'none',
        background: 'transparent', color: hw.textSecondary,
        fontSize: 11, cursor: 'pointer', borderRadius: hw.radius.sm,
      }}
      onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.05)' }}
      onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent' }}
    >
      <span style={{ flex: 1, textAlign: 'left' }}>{label}</span>
      {shortcut && <span style={{ fontSize: 9, color: hw.textFaint }}>{shortcut}</span>}
    </button>
  )
}

const sB: React.CSSProperties = {
  width: 17, height: 14, display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 7, fontWeight: 700, borderRadius: 6, padding: 0,
  border: `1px solid rgba(255,255,255,0.06)`,
}

const MAX_VISIBLE_SLOTS = 10

function FxSlots({ trackId, inserts }: { trackId?: string; inserts: InsertInfo[] }) {
  const { setInsertEnabled, removeFromTrack, reorderInsert, setFxChainBypassed, addToTrack } = usePluginStore()
  const { fetchTracks } = useTrackStore()
  const [menu, setMenu] = useState<{ x: number; y: number; slotId: string } | null>(null)
  const [dragOverSlotId, setDragOverSlotId] = useState<string | null>(null)
  const [dragOverEmptyIdx, setDragOverEmptyIdx] = useState<number | null>(null)

  useEffect(() => {
    if (!menu) return
    const close = (e: MouseEvent) => {
      const t = e.target as HTMLElement
      if (t.closest('[data-fx-menu]')) return
      setMenu(null)
    }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [menu])

  if (!trackId) {
    // Master strip keeps a purely decorative placeholder for layout parity.
    return (
      <div style={{ padding: '2px 3px', borderBottom: `1px solid ${hw.border}`, minHeight: 36 }} />
    )
  }

  const dragMime = {
    slot: 'application/x-hardwave-fx-slot',
    plugin: 'application/x-hardwave-plugin',
  }

  const handleDropOnSlot = async (targetIdx: number, e: React.DragEvent) => {
    e.preventDefault()
    const slotId = e.dataTransfer.getData(dragMime.slot)
    const pluginId = e.dataTransfer.getData(dragMime.plugin)
    if (slotId) {
      await reorderInsert(trackId, slotId, targetIdx)
      fetchTracks()
    } else if (pluginId) {
      // Replace: remove the existing slot at this index, then add at the same index.
      const existing = inserts[targetIdx]
      if (existing) {
        await removeFromTrack(trackId, existing.id)
      }
      const newSlotId = await addToTrack(trackId, pluginId)
      // The newly added slot lands at the end; move it to the target index.
      await reorderInsert(trackId, newSlotId, targetIdx)
      fetchTracks()
    }
    setDragOverSlotId(null)
    setDragOverEmptyIdx(null)
  }

  const handleDropOnEmpty = async (e: React.DragEvent) => {
    e.preventDefault()
    const pluginId = e.dataTransfer.getData(dragMime.plugin)
    if (pluginId) {
      await addToTrack(trackId, pluginId)
      fetchTracks()
    }
    setDragOverEmptyIdx(null)
  }

  const allBypassed = inserts.length > 0 && inserts.every(i => !i.enabled)
  const visibleCount = Math.max(inserts.length, 3)
  const paddingCount = Math.max(0, Math.min(MAX_VISIBLE_SLOTS, visibleCount) - inserts.length)

  return (
    <div style={{ padding: '2px 3px', borderBottom: `1px solid ${hw.border}` }}>
      {inserts.slice(0, MAX_VISIBLE_SLOTS).map((slot, idx) => {
        const isDragOver = dragOverSlotId === slot.id
        return (
          <div
            key={slot.id}
            data-testid={`fx-slot-${trackId}-${slot.id}`}
            draggable
            onDragStart={e => {
              e.dataTransfer.setData(dragMime.slot, slot.id)
              e.dataTransfer.effectAllowed = 'move'
            }}
            onDragOver={e => {
              if (e.dataTransfer.types.includes(dragMime.slot) ||
                  e.dataTransfer.types.includes(dragMime.plugin)) {
                e.preventDefault()
                setDragOverSlotId(slot.id)
              }
            }}
            onDragLeave={() => { if (dragOverSlotId === slot.id) setDragOverSlotId(null) }}
            onDrop={e => handleDropOnSlot(idx, e)}
            onClick={async () => {
              await setInsertEnabled(trackId, slot.id, !slot.enabled)
              fetchTracks()
            }}
            onContextMenu={e => {
              e.preventDefault()
              setMenu({ x: e.clientX, y: e.clientY, slotId: slot.id })
            }}
            title={`${slot.pluginName} — click to ${slot.enabled ? 'disable' : 'enable'} · drag to reorder`}
            style={{
              height: 11,
              background: isDragOver
                ? 'rgba(255, 200, 80, 0.25)'
                : slot.enabled ? 'rgba(124, 58, 237, 0.18)' : 'rgba(255,255,255,0.03)',
              border: `1px solid ${isDragOver
                ? 'rgba(255, 200, 80, 0.7)'
                : slot.enabled ? 'rgba(124, 58, 237, 0.4)' : hw.borderDark}`,
              borderRadius: hw.radius.sm, marginBottom: 1,
              display: 'flex', alignItems: 'center', padding: '0 3px', gap: 2,
              cursor: 'grab', overflow: 'hidden',
              opacity: slot.enabled ? 1 : 0.55,
            }}
          >
            <span style={{ fontSize: 7, color: hw.textFaint, flexShrink: 0 }}>{idx + 1}</span>
            <span style={{
              fontSize: 8, color: slot.enabled ? hw.textPrimary : hw.textMuted,
              overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1,
              textDecoration: slot.enabled ? 'none' : 'line-through',
            }}>{slot.pluginName}</span>
          </div>
        )
      })}
      {Array.from({ length: paddingCount }).map((_, i) => {
        const isDragOver = dragOverEmptyIdx === i
        return (
          <div
            key={`empty-${i}`}
            onDragOver={e => {
              if (e.dataTransfer.types.includes(dragMime.plugin)) {
                e.preventDefault()
                setDragOverEmptyIdx(i)
              }
            }}
            onDragLeave={() => { if (dragOverEmptyIdx === i) setDragOverEmptyIdx(null) }}
            onDrop={handleDropOnEmpty}
            style={{
              height: 11,
              background: isDragOver ? 'rgba(255, 200, 80, 0.2)' : 'rgba(255,255,255,0.02)',
              border: `1px dashed ${isDragOver ? 'rgba(255, 200, 80, 0.7)' : hw.borderDark}`,
              borderRadius: hw.radius.sm, marginBottom: 1,
              display: 'flex', alignItems: 'center', padding: '0 3px',
            }}
          >
            <span style={{ fontSize: 7, color: hw.textFaint }}>{inserts.length + i + 1}</span>
          </div>
        )
      })}

      {inserts.length > 0 && (
        <button
          onClick={async () => {
            await setFxChainBypassed(trackId, !allBypassed)
            fetchTracks()
          }}
          title={allBypassed ? 'Un-bypass FX chain' : 'Bypass entire FX chain'}
          data-testid={`fx-bypass-${trackId}`}
          style={{
            width: '100%', height: 10, marginTop: 2,
            fontSize: 7, fontWeight: 700, letterSpacing: 0.4,
            background: allBypassed ? 'rgba(255, 120, 80, 0.14)' : 'rgba(255,255,255,0.04)',
            color: allBypassed ? '#e08060' : hw.textMuted,
            border: `1px solid ${allBypassed ? 'rgba(255, 120, 80, 0.4)' : hw.borderDark}`,
            borderRadius: hw.radius.sm, cursor: 'pointer', padding: 0,
          }}
        >{allBypassed ? 'BYPASSED' : 'BYPASS'}</button>
      )}

      {menu && (
        <div
          data-fx-menu
          style={{
            position: 'fixed', left: menu.x, top: menu.y, zIndex: 100,
            background: '#151517', border: `1px solid ${hw.border}`, borderRadius: hw.radius.md,
            padding: 3, boxShadow: '0 6px 20px rgba(0,0,0,0.6)',
            display: 'flex', flexDirection: 'column', gap: 1, minWidth: 140,
          }}
        >
          {(() => {
            const slot = inserts.find(i => i.id === menu.slotId)
            const idx = inserts.findIndex(i => i.id === menu.slotId)
            if (!slot) return null
            return (
              <>
                <button
                  onClick={async () => {
                    await setInsertEnabled(trackId, slot.id, !slot.enabled)
                    fetchTracks()
                    setMenu(null)
                  }}
                  style={menuBtnStyle}
                >{slot.enabled ? 'Disable' : 'Enable'}</button>
                <button
                  disabled={idx <= 0}
                  onClick={async () => {
                    await reorderInsert(trackId, slot.id, idx - 1)
                    fetchTracks()
                    setMenu(null)
                  }}
                  style={{ ...menuBtnStyle, opacity: idx <= 0 ? 0.3 : 1 }}
                >Move up</button>
                <button
                  disabled={idx >= inserts.length - 1}
                  onClick={async () => {
                    await reorderInsert(trackId, slot.id, idx + 1)
                    fetchTracks()
                    setMenu(null)
                  }}
                  style={{ ...menuBtnStyle, opacity: idx >= inserts.length - 1 ? 0.3 : 1 }}
                >Move down</button>
                <div style={{ height: 1, background: hw.border, margin: '2px 0' }} />
                <button
                  onClick={async () => {
                    await removeFromTrack(trackId, slot.id)
                    fetchTracks()
                    setMenu(null)
                  }}
                  style={{ ...menuBtnStyle, color: '#e06060' }}
                >Remove</button>
              </>
            )
          })()}
        </div>
      )}
    </div>
  )
}

const menuBtnStyle: React.CSSProperties = {
  fontSize: 10, padding: '4px 8px', textAlign: 'left',
  background: 'transparent', border: 'none',
  color: hw.textPrimary, cursor: 'pointer', fontFamily: 'inherit',
  borderRadius: 3,
}
