import { memo, useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Knob } from '../../primitives/Knob'
import type { InsertInfo } from '../../../stores/trackStore'
import { usePluginPresetStore } from '../../../stores/pluginPresetStore'

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
  /// Preset dropdown anchor — click on plug-in name opens the list at
  /// the name's bounding rect. null = closed.
  const [presetMenuAnchor, setPresetMenuAnchor] = useState<DOMRect | null>(null)

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
      <button
        type="button"
        className="mx-fx-preset-arrow"
        title="Previous preset"
        onClick={(e) => {
          e.stopPropagation()
          usePluginPresetStore
            .getState()
            .step(trackId, insert.id, insert.pluginId, -1)
            .catch(console.error)
        }}
      >
        ‹
      </button>
      <div
        className="mx-fx-slot-name"
        title={insert.pluginName + ' · right-click for preset menu'}
        onClick={(e) => {
          // Click on the plug-in name opens the preset list dropdown.
          e.stopPropagation()
          const anchor = (e.currentTarget as HTMLElement).getBoundingClientRect()
          setPresetMenuAnchor(anchor)
        }}
      >
        {insert.pluginName}
      </div>
      <button
        type="button"
        className="mx-fx-preset-arrow"
        title="Next preset"
        onClick={(e) => {
          e.stopPropagation()
          usePluginPresetStore
            .getState()
            .step(trackId, insert.id, insert.pluginId, 1)
            .catch(console.error)
        }}
      >
        ›
      </button>
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
      {presetMenuAnchor && insert && (
        <PresetDropdown
          anchor={presetMenuAnchor}
          trackId={trackId}
          slotId={insert.id}
          pluginId={insert.pluginId}
          onClose={() => setPresetMenuAnchor(null)}
        />
      )}
    </div>
  )
})

// ---- Preset list dropdown ----
// Lazy-mounts: only opened when the user clicks the slot's plug-in name.
// On mount it refreshes the preset list for this plug-in (cheap — disk
// read + JSON parse), then renders a scrollable list of saved presets
// with click-to-load. Includes a "Save current as preset…" entry that
// snapshots get_state via the engine's existing snapshot mechanism.
function PresetDropdown(props: {
  anchor: DOMRect
  trackId: string
  slotId: string
  pluginId: string
  onClose: () => void
}) {
  const { anchor, trackId, slotId, pluginId, onClose } = props
  const ref = useRef<HTMLDivElement | null>(null)
  const presets = usePluginPresetStore((s) => s.byPlugin[pluginId] ?? null)
  const refresh = usePluginPresetStore((s) => s.refresh)
  const load = usePluginPresetStore((s) => s.load)
  const save = usePluginPresetStore((s) => s.save)

  useEffect(() => {
    refresh(pluginId).catch(console.error)
  }, [pluginId, refresh])

  // Outside click + Escape closes.
  useEffect(() => {
    const onPointerDown = (e: PointerEvent) => {
      if (!ref.current) return
      if (!ref.current.contains(e.target as Node)) onClose()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    const t = window.setTimeout(() => {
      window.addEventListener('pointerdown', onPointerDown)
    }, 0)
    window.addEventListener('keydown', onKey)
    return () => {
      window.clearTimeout(t)
      window.removeEventListener('pointerdown', onPointerDown)
      window.removeEventListener('keydown', onKey)
    }
  }, [onClose])

  // Clamp to viewport.
  const W = 220
  const vh = window.innerHeight
  const vw = window.innerWidth
  let left = anchor.left
  let top = anchor.bottom + 4
  if (left + W > vw - 8) left = vw - W - 8
  if (top + 240 > vh - 8) top = anchor.top - 240 - 4

  return (
    <div
      ref={ref}
      className="mx-fx-preset-menu"
      style={{ position: 'fixed', left, top, width: W }}
      onPointerDown={(e) => e.stopPropagation()}
    >
      <button
        type="button"
        className="mx-fx-preset-save"
        onClick={async () => {
          const name = window.prompt('Preset name:')?.trim()
          if (!name) return
          try {
            await save(trackId, slotId, pluginId, name)
          } catch (e) {
            console.error('save preset failed', e)
          }
          onClose()
        }}
      >
        + Save current as preset…
      </button>
      <div className="mx-fx-preset-list">
        {presets == null && <div className="mx-fx-preset-empty">Loading…</div>}
        {presets && presets.length === 0 && (
          <div className="mx-fx-preset-empty">No saved presets yet</div>
        )}
        {presets &&
          presets.map((p) => (
            <button
              key={p.id}
              type="button"
              className="mx-fx-preset-item"
              onClick={async () => {
                try {
                  await load(trackId, slotId, pluginId, p.id)
                } catch (e) {
                  console.error('load preset failed', e)
                }
                onClose()
              }}
              title={p.name}
            >
              {p.name}
            </button>
          ))}
      </div>
    </div>
  )
}
