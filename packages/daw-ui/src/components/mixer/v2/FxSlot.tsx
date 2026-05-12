import { memo, useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Knob } from '../../primitives/Knob'
import type { InsertInfo } from '../../../stores/trackStore'

export interface FxSlotProps {
  trackId: string
  /** 0-based slot position. Display uses (slotIndex + 1) padded. */
  slotIndex: number
  /** Undefined = empty slot (dashed placeholder + click-to-add). */
  insert?: InsertInfo
  /** Called when the user clicks an empty slot. Bubbles the slot's
   *  bounding rect so the flyout can anchor relative to it. */
  onOpenPicker: (slotIndex: number, anchor: DOMRect) => void
}

/**
 * One row in the FX rack. Two visual states:
 *
 *   - empty   → dashed LED + "+ click to add plugin"; click opens the
 *               favorites flyout anchored to this row.
 *   - filled  → solid LED (green=active, dim red=bypassed), plug-in
 *               name, dry/wet Knob (wet kind → "%" tooltip), wet %
 *               readout. Right-click → context menu.
 *
 * Engine wiring (verified against `src-tauri/src/commands/plugins.rs`):
 *   - Bypass / un-bypass → `set_insert_enabled` (line 428). The spec
 *     called it `bypass_plugin`; the real command toggles `enabled`.
 *   - Remove             → `remove_plugin_from_track` (line 400).
 *   - Show GUI           → `open_plugin_editor` (line 222) — requires a
 *     unique `window_label`; we use `${trackId}:${slotId}` so re-opening
 *     focuses the existing window.
 *   - Wet                → `set_insert_wet` (line 495) — backend clamps
 *     to 0..1, the Knob's `wet` kind labels as %, so we scale 0..1 ↔
 *     0..100 at the edges.
 *
 * `React.memo` matches the ChannelStrip discipline — the FxRackPanel
 * passes a stable `onOpenPicker` and slot props only change when the
 * underlying `InsertInfo` changes, so a wet drag on slot 03 doesn't
 * re-render slots 01/02/04..10.
 */
export const FxSlot = memo(function FxSlot(props: FxSlotProps) {
  const { trackId, slotIndex, insert, onOpenPicker } = props
  const rowRef = useRef<HTMLDivElement | null>(null)
  const [menuOpen, setMenuOpen] = useState(false)

  // Optimistic-local wet — same pattern as setVolumeLocal: drag mutates
  // a local mirror at 60fps, commit fires the IPC once on pointerup.
  const [localWet, setLocalWet] = useState<number | null>(null)
  const wet01 = localWet ?? insert?.wet ?? 1
  const wetPct = wet01 * 100

  const handleEmptyClick = useCallback(() => {
    if (insert) return
    const rect = rowRef.current?.getBoundingClientRect()
    if (rect) onOpenPicker(slotIndex, rect)
  }, [insert, onOpenPicker, slotIndex])

  const handleWetChange = useCallback((pct: number) => {
    setLocalWet(pct / 100)
  }, [])

  const handleWetCommit = useCallback(
    (pct: number) => {
      if (!insert) return
      const clamped = Math.max(0, Math.min(1, pct / 100))
      invoke('set_insert_wet', { trackId, slotId: insert.id, wet: clamped })
        .catch((e) => console.error('set_insert_wet failed', e))
        .finally(() => setLocalWet(null))
    },
    [insert, trackId],
  )

  // ---- context menu (right-click on filled slot) ----
  const onContextMenu = useCallback(
    (e: React.MouseEvent) => {
      if (!insert) return
      e.preventDefault()
      setMenuOpen(true)
    },
    [insert],
  )

  // Close the menu on any outside click / Escape.
  useEffect(() => {
    if (!menuOpen) return
    const onDown = (e: MouseEvent) => {
      if (!rowRef.current?.contains(e.target as Node)) setMenuOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setMenuOpen(false)
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey)
    }
  }, [menuOpen])

  const onBypass = useCallback(() => {
    if (!insert) return
    setMenuOpen(false)
    invoke('set_insert_enabled', {
      trackId,
      slotId: insert.id,
      enabled: !insert.enabled,
    }).catch((e) => console.error('set_insert_enabled failed', e))
  }, [insert, trackId])

  const onReplace = useCallback(() => {
    if (!insert) return
    setMenuOpen(false)
    const rect = rowRef.current?.getBoundingClientRect()
    if (rect) onOpenPicker(slotIndex, rect)
  }, [insert, onOpenPicker, slotIndex])

  const onRemove = useCallback(() => {
    if (!insert) return
    setMenuOpen(false)
    invoke('remove_plugin_from_track', { trackId, slotId: insert.id }).catch(
      (e) => console.error('remove_plugin_from_track failed', e),
    )
  }, [insert, trackId])

  const onShowGui = useCallback(() => {
    if (!insert) return
    setMenuOpen(false)
    // Unique label per slot so a re-click focuses the existing window
    // instead of erroring. plugins.rs:270 explicitly handles this.
    const windowLabel = `plugin-editor:${trackId}:${insert.id}`
    invoke('open_plugin_editor', {
      pluginId: insert.pluginId,
      windowLabel,
      trackId,
      slotId: insert.id,
    }).catch((e) => console.error('open_plugin_editor failed', e))
  }, [insert, trackId])

  const idxLabel = String(slotIndex + 1).padStart(2, '0')

  if (!insert) {
    return (
      <div
        ref={rowRef}
        className="mx-fx-slot empty"
        data-slot-index={slotIndex}
        onClick={handleEmptyClick}
      >
        <div className="mx-fx-slot-idx">{idxLabel}</div>
        <div className="mx-fx-slot-led" />
        <div className="mx-fx-slot-name">+ click to add plugin</div>
        <div />
        <div className="mx-fx-slot-wet">—</div>
      </div>
    )
  }

  const bypassed = !insert.enabled

  return (
    <div
      ref={rowRef}
      className={'mx-fx-slot' + (bypassed ? ' bypass' : '')}
      data-slot-index={slotIndex}
      onContextMenu={onContextMenu}
    >
      <div className="mx-fx-slot-idx">{idxLabel}</div>
      <div className="mx-fx-slot-led" />
      <div className="mx-fx-slot-name" title={insert.pluginName}>
        {insert.pluginName}
      </div>
      <div className="mx-fx-slot-knob">
        <Knob
          value={wetPct}
          min={0}
          max={100}
          defaultValue={100}
          kind="wet"
          size={18}
          onChange={handleWetChange}
          onChangeEnd={handleWetCommit}
          title="Dry / Wet"
        />
      </div>
      <div className="mx-fx-slot-wet">{Math.round(wetPct)}%</div>
      {menuOpen && (
        <div className="mx-fx-slot-menu" role="menu">
          <button role="menuitem" onClick={onBypass}>
            {bypassed ? 'Un-bypass' : 'Bypass'}
          </button>
          <button role="menuitem" onClick={onReplace}>
            Replace…
          </button>
          <button role="menuitem" onClick={onRemove}>
            Remove
          </button>
          <button role="menuitem" onClick={onShowGui} disabled={!insert}>
            Show GUI
          </button>
        </div>
      )}
    </div>
  )
})
