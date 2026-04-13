import { useRef, useState } from 'react'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'
import { useTransportStore } from '../../stores/transportStore'

export function TrackList() {
  const {
    tracks, selectedTrackId, selectTrack,
    toggleMute, toggleSolo, toggleArm, reorderTrack,
    trackHeights, setTrackHeight,
  } = useTrackStore()
  const defaultHeight = useTransportStore(s => s.trackHeight)
  const audioTracks = tracks.filter(t => t.kind !== 'Master')

  const [dragId, setDragId] = useState<string | null>(null)
  const [dragOverIdx, setDragOverIdx] = useState<number | null>(null)
  const resizingRef = useRef<{ id: string; startY: number; startH: number } | null>(null)

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
                <span style={{
                  fontSize: 10, color: hw.textPrimary, fontWeight: 500,
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                }}>
                  {track.name}
                </span>
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
    </div>
  )
}

const tb: React.CSSProperties = {
  width: 18, height: 14,
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  fontSize: 8, fontWeight: 700,
  border: `1px solid rgba(255,255,255,0.06)`,
  borderRadius: 6, padding: 0,
}
