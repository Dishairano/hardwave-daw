import { memo, useEffect, useMemo, useRef } from 'react'
import { usePluginCatalogStore, isHardwaveNative } from '../../stores/pluginCatalogStore'
import { usePluginFavoritesStore } from '../../stores/pluginFavoritesStore'
import './picker.css'

export interface PluginPickerFlyoutProps {
  /** Bounding rect of the clicked empty slot — we anchor to its left edge. */
  anchor: DOMRect
  /** 0-based slot index, surfaced in the header as "Slot N". */
  slotIndex: number
  /** Called when the user clicks a favorite. Receives the plug-in id. */
  onPick: (pluginId: string) => void
  /** Called when the user clicks the "Search more plug-ins…" footer. */
  onSearchMore: () => void
  /** Called when the user clicks outside the flyout or presses Esc. */
  onClose: () => void
}

const FLYOUT_W = 260
/** Worst-case flyout height — used to pin the popover inside the viewport. */
const FLYOUT_MAX_H = 320

/**
 * Favorites popover anchored to the clicked empty slot.
 *
 * Position policy: prefer to the left of the slot (the FX rack lives on
 * the right side of the screen, so leftward keeps the popover on
 * canvas). If that would clip the viewport, swap to the right of the
 * slot. Vertical position is the slot's top, clamped to the viewport.
 *
 * Shows up to 7 entries from `pluginFavoritesStore.getFlyoutList()`,
 * resolved through `pluginCatalogStore.byId()`. Catalog entries that
 * have disappeared (e.g. plug-in was uninstalled between sessions) are
 * silently filtered out — the user shouldn't be offered a dead pick.
 *
 * Closes on:
 *   - click outside (window-level mousedown handler)
 *   - Escape
 *   - parent unmounts it (e.g. switching tracks while open)
 */
export const PluginPickerFlyout = memo(function PluginPickerFlyout(
  props: PluginPickerFlyoutProps,
) {
  const { anchor, slotIndex, onPick, onSearchMore, onClose } = props
  const rootRef = useRef<HTMLDivElement | null>(null)

  // Subscribe so the flyout repopulates if the catalog finishes loading
  // while the flyout is open (first-ever picker open + cold catalog).
  const plugins = usePluginCatalogStore((s) => s.plugins)
  // `pinned` and `recent` are arrays — selecting them directly keeps
  // the subscription fine-grained.
  const pinned = usePluginFavoritesStore((s) => s.pinned)
  const recent = usePluginFavoritesStore((s) => s.recent)

  const favs = useMemo(() => {
    // Inline the getFlyoutList logic so the memo depends on pinned/recent
    // directly — the store's bare method reference doesn't change identity
    // even when its inputs change, which would defeat memoization.
    const seen = new Set<string>()
    const out: string[] = []
    for (const id of pinned) {
      if (!seen.has(id)) { seen.add(id); out.push(id) }
      if (out.length >= 7) break
    }
    if (out.length < 7) {
      for (const id of recent) {
        if (!seen.has(id)) { seen.add(id); out.push(id) }
        if (out.length >= 7) break
      }
    }
    return out
      .map((id) => plugins.find((p) => p.id === id))
      .filter((p): p is NonNullable<typeof p> => !!p)
  }, [pinned, recent, plugins])

  // Position: prefer left-of-slot. Recomputed once at mount + on anchor
  // change. Clamped horizontally and vertically to keep the popover in
  // the viewport.
  const style = useMemo<React.CSSProperties>(() => {
    const vw = window.innerWidth
    const vh = window.innerHeight
    let left = anchor.left - FLYOUT_W - 8
    if (left < 12) left = anchor.right + 8
    if (left + FLYOUT_W > vw - 12) left = vw - FLYOUT_W - 12
    const top = Math.max(12, Math.min(vh - FLYOUT_MAX_H, anchor.top))
    return { left, top, width: FLYOUT_W }
  }, [anchor])

  // Click-outside + Escape. Use mousedown so a click on the slot it was
  // opened from (which fires onClick AFTER mousedown) doesn't immediately
  // re-trigger the open — by then this flyout is already unmounted.
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current) return
      if (rootRef.current.contains(e.target as Node)) return
      onClose()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey)
    }
  }, [onClose])

  return (
    <div ref={rootRef} className="hw-pf open" style={style} role="dialog" aria-label="Add plug-in">
      <div className="hw-pf-head">
        <span>Add plug-in</span>
        <b>Slot {slotIndex + 1}</b>
      </div>
      <div className="hw-pf-list">
        {favs.length === 0 && (
          <div className="hw-pf-empty">No favorites yet — search more →</div>
        )}
        {favs.map((p) => {
          const hw = isHardwaveNative(p)
          return (
            <div
              key={p.id}
              className={'hw-pf-item' + (hw ? ' hw' : '')}
              onClick={() => onPick(p.id)}
            >
              <span className="hw-pf-icn" aria-hidden="true">{hw ? 'H' : '·'}</span>
              <span className="hw-pf-text">
                <span className="hw-pf-name">{p.name}</span>
                <span className="hw-pf-cat">{p.vendor} · {p.category}</span>
              </span>
            </div>
          )
        })}
      </div>
      <div className="hw-pf-more" onClick={onSearchMore} role="button">
        <span>Search more plug-ins…</span>
        <span className="hw-pf-more-ico" aria-hidden="true">›</span>
      </div>
    </div>
  )
})
