import { useEffect, useRef, useState } from 'react'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'
import { useTransportStore } from '../../stores/transportStore'
import { PATTERN_COLORS } from '../../stores/patternStore'

export function TrackList() {
  const {
    tracks, selectedTrackId, selectTrack,
    toggleMute, toggleSolo, toggleArm, reorderTrack,
    trackHeights, setTrackHeight,
    renameTrack, setTrackColor, removeTrack,
  } = useTrackStore()
  const defaultHeight = useTransportStore(s => s.trackHeight)
  const audioTracks = tracks.filter(t => t.kind !== 'Master')

  const [dragId, setDragId] = useState<string | null>(null)
  const [dragOverIdx, setDragOverIdx] = useState<number | null>(null)
  const resizingRef = useRef<{ id: string; startY: number; startH: number } | null>(null)
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; trackId: string } | null>(null)
  const [renaming, setRenaming] = useState<{ id: string; draft: string } | null>(null)
  const renameInputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (!ctxMenu) return
    const close = (e: MouseEvent) => {
      const t = e.target as HTMLElement
      if (t.closest('[data-track-ctx-menu]')) return
      setCtxMenu(null)
    }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [ctxMenu])

  useEffect(() => { if (renaming) renameInputRef.current?.select() }, [renaming])

  const commitRename = async () => {
    if (!renaming) return
    const trimmed = renaming.draft.trim()
    const original = audioTracks.find(t => t.id === renaming.id)
    if (trimmed && original && trimmed !== original.name) {
      await renameTrack(renaming.id, trimmed)
    }
    setRenaming(null)
  }

  const onResizeStart = (id: string, e: React.PointerEvent) => {
    e.stopPropagation()
    e.preventDefault()
    const startH = trackHeights[id] ?? defaultHeight
    resizingRef.current = { id, startY: e.clientY, startH }
    const onMove = (ev: PointerEvent) => {
      const r = resizingRef.current
      if (!r) return
      setTrackHeight(r.id, r.startH + (ev.clientY - r.startY))
    }
    const onUp = () => {
      resizingRef.current = null
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
  }

  return (
    <div style={{
      width: 155, minWidth: 155,
      background: 'rgba(255,255,255,0.02)',
      backdropFilter: hw.blur.sm,
      borderRight: `1px solid ${hw.border}`,
      display: 'flex', flexDirection: 'column',
    }}>
      <div style={{
        height: 22, background: 'rgba(255,255,255,0.01)', borderBottom: `1px solid ${hw.border}`,
        display: 'flex', alignItems: 'center', padding: '0 8px',
      }}>
        <span style={{ fontSize: 10, color: hw.textMuted }}>Playlist</span>
      </div>

      <div style={{ flex: 1, overflowY: 'auto' }}>
        {audioTracks.map((track, idx) => {
          const h = trackHeights[track.id] ?? defaultHeight
          const isDragOver = dragOverIdx === idx && dragId && dragId !== track.id
          return (
          <div
            key={track.id}
            draggable
            onDragStart={e => {
              setDragId(track.id)
              e.dataTransfer.effectAllowed = 'move'
            }}
            onDragOver={e => {
              e.preventDefault()
              e.dataTransfer.dropEffect = 'move'
              setDragOverIdx(idx)
            }}
            onDragLeave={() => setDragOverIdx(prev => (prev === idx ? null : prev))}
            onDrop={e => {
              e.preventDefault()
              if (dragId && dragId !== track.id) {
                reorderTrack(dragId, idx)
              }
              setDragId(null)
              setDragOverIdx(null)
            }}
            onDragEnd={() => { setDragId(null); setDragOverIdx(null) }}
            onClick={() => selectTrack(track.id)}
            onContextMenu={e => {
              e.preventDefault()
              setCtxMenu({ x: e.clientX, y: e.clientY, trackId: track.id })
              selectTrack(track.id)
            }}
            style={{
              height: h, display: 'flex', alignItems: 'stretch',
              borderBottom: `1px solid ${hw.border}`,
              borderTop: isDragOver ? `2px solid ${hw.accent}` : undefined,
              background: selectedTrackId === track.id
                ? hw.accentDim
                : idx % 2 === 0 ? 'transparent' : 'rgba(255,255,255,0.015)',
              cursor: 'default',
              transition: 'background 0.1s',
              opacity: dragId === track.id ? 0.5 : 1,
              position: 'relative',
            }}
          >
            <div style={{ width: 3, background: track.color, flexShrink: 0 }} />
            <div style={{ flex: 1, padding: '5px 7px', display: 'flex', flexDirection: 'column', justifyContent: 'center' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                <span style={{ fontSize: 9, color: hw.textFaint }}>{idx + 1}</span>
                {renaming?.id === track.id ? (
                  <input
                    ref={renameInputRef}
                    value={renaming.draft}
                    onChange={e => setRenaming({ id: track.id, draft: e.target.value })}
                    onClick={e => e.stopPropagation()}
                    onBlur={commitRename}
                    onKeyDown={e => {
                      if (e.key === 'Enter') commitRename()
                      else if (e.key === 'Escape') setRenaming(null)
                    }}
                    style={{
                      flex: 1, minWidth: 0, fontSize: 10, fontWeight: 500,
                      color: hw.textPrimary, background: 'rgba(255,255,255,0.08)',
                      border: `1px solid ${hw.accent}`, borderRadius: 2, padding: '1px 3px',
                      outline: 'none',
                    }}
                  />
                ) : (
                  <span
                    onDoubleClick={e => { e.stopPropagation(); setRenaming({ id: track.id, draft: track.name }) }}
                    style={{
                      fontSize: 10, color: hw.textPrimary, fontWeight: 500,
                      overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                    }}
                  >
                    {track.name}
                  </span>
                )}
              </div>
              <div style={{ display: 'flex', gap: 3, marginTop: 5 }}>
                <button
                  onClick={e => { e.stopPropagation(); toggleMute(track.id) }}
                  style={{
                    ...tb,
                    color: track.muted ? hw.red : hw.textMuted,
                    background: track.muted ? hw.redDim : 'rgba(255,255,255,0.04)',
                  }}
                >M</button>
                <button
                  onClick={e => { e.stopPropagation(); toggleSolo(track.id) }}
                  style={{
                    ...tb,
                    color: track.soloed ? hw.yellow : hw.textMuted,
                    background: track.soloed ? hw.yellowDim : 'rgba(255,255,255,0.04)',
                  }}
                >S</button>
                <button
                  onClick={e => { e.stopPropagation(); toggleArm(track.id) }}
                  title="Arm for recording"
                  style={{
                    ...tb,
                    color: track.armed ? hw.red : hw.textMuted,
                    background: track.armed ? hw.redDim : 'rgba(255,255,255,0.04)',
                  }}
                >R</button>
                <span style={{ fontSize: 8, color: hw.textFaint, marginLeft: 'auto', alignSelf: 'center' }}>
                  {track.volume_db > -60 ? `${track.volume_db.toFixed(0)}dB` : '-\u221E'}
                </span>
              </div>
            </div>
            <div
              onPointerDown={e => onResizeStart(track.id, e)}
              title="Drag to resize"
              style={{
                position: 'absolute', left: 0, right: 0, bottom: 0,
                height: 4, cursor: 'ns-resize',
                background: 'transparent',
              }}
            />
          </div>
        )})}

        {audioTracks.length === 0 && (
          <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 10 }}>
            Add tracks from toolbar
          </div>
        )}
      </div>
      {ctxMenu && (() => {
        const t = audioTracks.find(x => x.id === ctxMenu.trackId)
        if (!t) return null
        return (
          <div
            data-track-ctx-menu
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
              {t.name}
            </div>
            <TrackMenuItem label="Rename" shortcut="F2" onClick={() => {
              setCtxMenu(null)
              setRenaming({ id: t.id, draft: t.name })
            }} />
            <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
            <div style={{ padding: '4px 8px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
              Color
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(6, 1fr)', gap: 2, padding: '2px 6px 4px' }}>
              {PATTERN_COLORS.map(c => (
                <button key={c} title={c}
                  onClick={async () => {
                    setCtxMenu(null)
                    await setTrackColor(t.id, c)
                  }}
                  style={{
                    width: 18, height: 18, borderRadius: hw.radius.sm,
                    background: c, border: c === t.color ? '2px solid #fff' : '1px solid rgba(255,255,255,0.12)',
                    cursor: 'pointer', padding: 0,
                  }}
                />
              ))}
            </div>
            <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
            <TrackMenuItem label="Delete" shortcut="Del" danger onClick={async () => {
              setCtxMenu(null)
              await removeTrack(t.id)
            }} />
          </div>
        )
      })()}
    </div>
  )
}

function TrackMenuItem({ label, shortcut, danger, onClick }: {
  label: string; shortcut?: string; danger?: boolean; onClick: () => void
}) {
  return (
    <button
      onClick={onClick}
      style={{
        width: '100%', display: 'flex', alignItems: 'center',
        padding: '5px 8px', gap: 8, border: 'none',
        background: 'transparent', color: danger ? hw.red : hw.textSecondary,
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

const tb: React.CSSProperties = {
  width: 18, height: 14,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 8, fontWeight: 700,
  border: `1px solid rgba(255,255,255,0.06)`,
  borderRadius: 6, padding: 0,
}
