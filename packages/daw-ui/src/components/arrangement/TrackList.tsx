import { useEffect, useRef, useState } from 'react'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'
import { useTransportStore } from '../../stores/transportStore'
import { PATTERN_COLORS } from '../../stores/patternStore'
import { useTrackTemplateStore } from '../../stores/trackTemplateStore'
import { useTrackFolderStore, type TrackFolder } from '../../stores/trackFolderStore'
import { useNotificationStore } from '../../stores/notificationStore'
import { DetachButton } from '../FloatingWindow'

export function TrackList() {
  const {
    tracks, selectedTrackId, selectTrack,
    toggleMute, toggleSolo, toggleSoloSafe, toggleArm, reorderTrack,
    trackHeights, setTrackHeight,
    renameTrack, setTrackColor, removeTrack,
  } = useTrackStore()
  const defaultHeight = useTransportStore(s => s.trackHeight)
  const audioTracks = tracks.filter(t => t.kind !== 'Master')
  const folders = useTrackFolderStore(s => s.folders)
  const toggleCollapsed = useTrackFolderStore(s => s.toggleCollapsed)
  const createFolder = useTrackFolderStore(s => s.createFolder)
  const deleteFolder = useTrackFolderStore(s => s.deleteFolder)
  const renameFolder = useTrackFolderStore(s => s.renameFolder)
  const setFolderColor = useTrackFolderStore(s => s.setFolderColor)
  const addTrackToFolder = useTrackFolderStore(s => s.addTrackToFolder)
  const removeTrackFromFolder = useTrackFolderStore(s => s.removeTrackFromFolder)

  const folderByTrack = new Map<string, TrackFolder>()
  for (const f of folders) for (const tid of f.trackIds) folderByTrack.set(tid, f)
  const shownSeqFolderIds = new Set<string>()
  const rows: Array<{ kind: 'folderHeader'; folder: TrackFolder } | { kind: 'track'; trackIdx: number }> = []
  audioTracks.forEach((t, trackIdx) => {
    const f = folderByTrack.get(t.id)
    if (f && !shownSeqFolderIds.has(f.id)) {
      rows.push({ kind: 'folderHeader', folder: f })
      shownSeqFolderIds.add(f.id)
    }
    if (!f || !f.collapsed) rows.push({ kind: 'track', trackIdx })
  })

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
        display: 'flex', alignItems: 'center', padding: '0 8px', gap: 4,
      }}>
        <span style={{ fontSize: 10, color: hw.textMuted }}>Playlist</span>
        <div style={{ flex: 1 }} />
        {audioTracks.some(t => t.armed) && (
          <button
            onClick={async () => {
              for (const t of audioTracks) {
                if (t.armed) await toggleArm(t.id)
              }
            }}
            title="Disarm all armed tracks"
            style={{
              height: 14, padding: '0 6px', fontSize: 8, fontWeight: 700,
              color: hw.red, background: hw.redDim,
              border: `1px solid ${hw.red}`, borderRadius: hw.radius.sm, cursor: 'pointer',
            }}
          >DISARM</button>
        )}
        <DetachButton panelId="playlist" />
      </div>

      <div style={{ flex: 1, overflowY: 'auto' }}>
        {rows.map((row, rowKey) => {
          if (row.kind === 'folderHeader') {
            const f = row.folder
            return (
              <div
                key={`fld_${f.id}`}
                onClick={() => toggleCollapsed(f.id)}
                onDoubleClick={e => {
                  e.stopPropagation()
                  const next = window.prompt('Folder name:', f.name)?.trim()
                  if (next) renameFolder(f.id, next)
                }}
                onContextMenu={e => {
                  e.preventDefault()
                  const next = window.prompt(
                    `Folder "${f.name}" — enter new name, a 6-digit hex color (e.g. #A855F7), or leave blank to delete the folder:`,
                    f.name,
                  )
                  if (next == null) return
                  const trimmed = next.trim()
                  if (trimmed === '') { deleteFolder(f.id); return }
                  if (/^#[0-9a-fA-F]{6}$/.test(trimmed)) { setFolderColor(f.id, trimmed); return }
                  renameFolder(f.id, trimmed)
                }}
                style={{
                  display: 'flex', alignItems: 'center', gap: 5,
                  padding: '3px 6px',
                  borderBottom: `1px solid ${hw.border}`,
                  background: `linear-gradient(90deg, ${f.color}22, transparent 80%)`,
                  cursor: 'pointer',
                }}
                title="Click to expand/collapse. Double-click to rename. Right-click for name/color/delete."
              >
                <svg width="8" height="8" viewBox="0 0 8 8" style={{ transform: f.collapsed ? 'rotate(-90deg)' : 'none', transition: 'transform 0.12s' }}>
                  <path d="M1 2l3 3.5L7 2" fill="none" stroke={f.color} strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
                <div style={{ width: 4, height: 14, background: f.color, borderRadius: 1 }} />
                <span style={{ fontSize: 10, fontWeight: 600, color: hw.textPrimary, flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {f.name}
                </span>
                <span style={{ fontSize: 8, color: hw.textFaint }}>
                  {f.trackIds.length}
                </span>
              </div>
            )
          }
          const track = audioTracks[row.trackIdx]
          const idx = row.trackIdx
          const inFolder = folderByTrack.get(track.id) != null
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
              paddingLeft: inFolder ? 8 : 0,
            }}
          >
            {/* 2 px color stripe (mockup pattern, was 3 px) */}
            <div style={{ width: 2, background: track.color, flexShrink: 0 }} />
            <div style={{ flex: 1, padding: '4px 7px', display: 'flex', flexDirection: 'column', justifyContent: 'center', minWidth: 0 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
                <span style={{
                  fontFamily: hw.font.mono, fontSize: 8, fontWeight: 500,
                  color: hw.textFaint, letterSpacing: hw.tracking.wide,
                }}>{idx + 1}</span>
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
                      flex: 1, minWidth: 0,
                      fontFamily: hw.font.mono, fontSize: 9, fontWeight: 500,
                      color: hw.textPrimary, background: 'rgba(255,255,255,0.08)',
                      border: `1px solid ${hw.accent}`, borderRadius: 2, padding: '1px 3px',
                      outline: 'none',
                    }}
                  />
                ) : (
                  <span
                    onDoubleClick={e => { e.stopPropagation(); setRenaming({ id: track.id, draft: track.name }) }}
                    style={{
                      fontFamily: hw.font.mono, fontSize: 9,
                      color: track.muted ? hw.textFaint : hw.textPrimary,
                      fontWeight: 500, letterSpacing: '0.01em',
                      overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                      flex: 1, minWidth: 0,
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
                  title={track.solo_safe ? 'Solo (solo-safe on: ignores other solos)' : 'Solo'}
                  style={{
                    ...tb,
                    color: track.soloed ? hw.yellow : hw.textMuted,
                    background: track.soloed ? hw.yellowDim : 'rgba(255,255,255,0.04)',
                    outline: track.solo_safe ? `1px solid ${hw.accent}` : undefined,
                    outlineOffset: track.solo_safe ? -2 : undefined,
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

        {/* Empty placeholder slots — fill out the playlist to 500 rows like the mockup */}
        {(() => {
          const TOTAL_SLOTS = 500
          const realCount = audioTracks.length
          const placeholderCount = Math.max(0, TOTAL_SLOTS - realCount)
          const slotHeight = defaultHeight
          const placeholders = []
          for (let i = 0; i < placeholderCount; i++) {
            const slotNum = realCount + i + 1
            placeholders.push(
              <div
                key={`slot_${slotNum}`}
                style={{
                  height: slotHeight,
                  display: 'flex',
                  alignItems: 'center',
                  borderBottom: `1px solid #0a0a0e`,
                  background: (realCount + i) % 2 === 0 ? '#08080c' : '#0b0b10',
                  paddingLeft: 2,
                }}
              >
                <div style={{ width: 2, height: '100%', background: 'transparent', flexShrink: 0 }} />
                <span style={{
                  width: 22, textAlign: 'right',
                  fontFamily: hw.font.mono, fontSize: 8, fontWeight: 500,
                  color: '#3f3f46', letterSpacing: hw.tracking.wide,
                  paddingRight: 5, paddingLeft: 5,
                }}>{slotNum}</span>
              </div>,
            )
          }
          return placeholders
        })()}
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
            <TrackMenuItem label={t.muted ? 'Unmute' : 'Mute'} onClick={async () => {
              setCtxMenu(null)
              await toggleMute(t.id)
            }} />
            <TrackMenuItem label={t.soloed ? 'Unsolo' : 'Solo'} onClick={async () => {
              setCtxMenu(null)
              await toggleSolo(t.id)
            }} />
            <TrackMenuItem label={t.solo_safe ? 'Disable solo-safe' : 'Solo-safe'} onClick={async () => {
              setCtxMenu(null)
              await toggleSoloSafe(t.id)
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
            <div style={{ padding: '4px 8px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
              Folder
            </div>
            <TrackMenuItem label="New folder from this track" onClick={() => {
              setCtxMenu(null)
              const name = window.prompt('Folder name:', `Group ${folders.length + 1}`)?.trim()
              if (!name) return
              createFolder([t.id], name)
            }} />
            {folders.length > 0 && (() => {
              const currentFolder = folderByTrack.get(t.id)
              const targets = folders.filter(f => f.id !== currentFolder?.id)
              if (targets.length === 0) return null
              return (
                <>
                  {targets.map(f => (
                    <TrackMenuItem
                      key={`addto_${f.id}`}
                      label={`Add to "${f.name}"`}
                      onClick={() => {
                        setCtxMenu(null)
                        addTrackToFolder(t.id, f.id)
                      }}
                    />
                  ))}
                </>
              )
            })()}
            {folderByTrack.get(t.id) && (
              <TrackMenuItem label="Remove from folder" onClick={() => {
                setCtxMenu(null)
                removeTrackFromFolder(t.id)
              }} />
            )}
            <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
            <TrackMenuItem label="Save as track template…" onClick={() => {
              setCtxMenu(null)
              const name = window.prompt('Template name:', t.name)?.trim()
              if (!name) return
              const kind = t.kind === 'Midi' ? 'Midi' : 'Audio'
              useTrackTemplateStore.getState().save({
                name,
                kind,
                trackName: t.name,
                color: t.color,
                volumeDb: t.volume_db,
                pan: t.pan,
              })
              useNotificationStore.getState().push('info', `Saved track template "${name}"`)
            }} />
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
  width: 13, height: 11,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontFamily: "'JetBrains Mono', ui-monospace, Menlo, monospace",
  fontSize: 7, fontWeight: 700,
  border: `1px solid rgba(255,255,255,0.08)`,
  borderRadius: 2, padding: 0,
  cursor: 'default',
}
