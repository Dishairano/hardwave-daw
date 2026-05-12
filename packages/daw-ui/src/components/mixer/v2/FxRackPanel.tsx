import { memo, Suspense, lazy, useCallback, useState } from 'react'
import { useMixerSelectionStore } from '../../../stores/mixerSelectionStore'
import { useTrackById } from '../../../stores/trackStore'
import { usePluginFavoritesStore } from '../../../stores/pluginFavoritesStore'
import { usePluginCatalogStore } from '../../../stores/pluginCatalogStore'
import { RoutingMatrix } from './RoutingMatrix'
import { FxSlot } from './FxSlot'
import { PluginPickerFlyout } from '../../plugin-picker/PluginPickerFlyout'
import { invoke } from '@tauri-apps/api/core'

// Lazy: the modal pulls in the category sidebar + full plug-in list
// renderer. It only matters when the user clicks "Search more…", so
// keep it out of the initial mixer bundle. Suspense fallback is `null`
// because the click already dismissed the flyout — the modal appears
// the moment the chunk arrives.
const PluginPickerModal = lazy(() =>
  import('../../plugin-picker/PluginPickerModal').then((m) => ({
    default: m.PluginPickerModal,
  })),
)

/** Total slot count per track. Mirrors the mockup's "10 of 10". */
const SLOT_COUNT = 10

/**
 * 360 px right column of the mixer. Replaces the Phase 1 stub. Composed
 * of three regions:
 *
 *   1. fx-head      — track number badge + name + role line.
 *   2. RoutingMatrix — send list for the selected track.
 *   3. 10 × FxSlot   — inserts[i] for i in 0..9 (undefined where empty).
 *
 * Selection is read from `mixerSelectionStore.selectedTrackId`. The
 * MixerPanel seeds the selection with the master id on mount, so the
 * rack always has a sensible target. When `selectedTrackId === null` we
 * render a "no selection" placeholder (defensive — shouldn't happen
 * after the seed effect runs).
 *
 * Picker state lives here on purpose: the flyout's anchor rect is owned
 * by whichever FxSlot was just clicked, so the FxRackPanel is the
 * lowest common ancestor that can route picker actions back to the
 * right slot.
 */
export const FxRackPanel = memo(function FxRackPanel() {
  const selectedId = useMixerSelectionStore((s) => s.selectedTrackId)
  const track = useTrackById(selectedId ?? '')

  // Picker state — anchor rect + slot index of the click target. Both
  // null when closed. The modal stays "open" without an anchor since it
  // covers the whole viewport.
  const [pickerAnchor, setPickerAnchor] = useState<DOMRect | null>(null)
  const [pickerSlotIndex, setPickerSlotIndex] = useState<number | null>(null)
  const [modalOpen, setModalOpen] = useState(false)

  const onOpenPicker = useCallback((slotIndex: number, anchor: DOMRect) => {
    // Kick a lazy catalog scan the first time the picker opens. No-op
    // on subsequent opens.
    usePluginCatalogStore.getState().ensureLoaded()
    setPickerSlotIndex(slotIndex)
    setPickerAnchor(anchor)
  }, [])

  const closeFlyout = useCallback(() => {
    setPickerAnchor(null)
  }, [])

  const closeAll = useCallback(() => {
    setPickerAnchor(null)
    setPickerSlotIndex(null)
    setModalOpen(false)
  }, [])

  const onSearchMore = useCallback(() => {
    setPickerAnchor(null)
    setModalOpen(true)
  }, [])

  const onPick = useCallback(
    (pluginId: string) => {
      if (!track) {
        closeAll()
        return
      }
      // Optimistic UX: mark used + close immediately. The audio thread
      // load happens behind the scenes; trackStore.fetchTracks() will
      // pull in the new InsertInfo on the next tick.
      usePluginFavoritesStore.getState().markUsed(pluginId)
      const targetTrackId = track.id
      closeAll()
      invoke('add_plugin_to_track', { trackId: targetTrackId, pluginId })
        .catch((e) => console.error('add_plugin_to_track failed', e))
    },
    [track, closeAll],
  )

  if (!track) {
    return (
      <div className="mx-fx-rack">
        <div className="mx-fx-rack-empty">No track selected</div>
      </div>
    )
  }

  // Slot index label in the header — `M` for master, otherwise we
  // surface the track index within its kind. We don't have a global
  // 1-based "insert number" in trackStore, so fall back to the first
  // letter of the kind for the badge (matches the s-num convention).
  const isMaster = track.kind === 'Master'
  const badge = isMaster ? 'M' : (track.name.match(/\d+/)?.[0] ?? track.kind[0] ?? '·')
  const roleLine = isMaster
    ? 'Master · Stereo · final output'
    : `${track.kind} · ${track.muted ? 'muted' : 'active'}`

  // Pad inserts up to SLOT_COUNT so the row count is always 10.
  const insertsPadded = Array.from({ length: SLOT_COUNT }, (_, i) => track.inserts[i])
  const activeCount = track.inserts.filter((s) => s.enabled).length

  return (
    <div className="mx-fx-rack">
      <div className="mx-fx-head">
        <div className="mx-fx-head-target">
          <div className="mx-fx-head-num">{badge}</div>
          <div>
            <div className="mx-fx-head-name">{track.name}</div>
            <div className="mx-fx-head-sub">{roleLine}</div>
          </div>
        </div>
      </div>

      <RoutingMatrix trackId={track.id} />

      <div className="mx-fx-slots">
        <h4>
          FX Slots <span>{SLOT_COUNT} of {SLOT_COUNT} · {activeCount} active</span>
        </h4>
        {insertsPadded.map((insert, i) => (
          <FxSlot
            key={insert?.id ?? `empty-${i}`}
            trackId={track.id}
            slotIndex={i}
            insert={insert}
            onOpenPicker={onOpenPicker}
          />
        ))}
      </div>

      {pickerAnchor !== null && pickerSlotIndex !== null && (
        <PluginPickerFlyout
          anchor={pickerAnchor}
          slotIndex={pickerSlotIndex}
          onPick={onPick}
          onSearchMore={onSearchMore}
          onClose={closeFlyout}
        />
      )}

      {modalOpen && (
        <Suspense fallback={null}>
          <PluginPickerModal
            slotIndex={pickerSlotIndex ?? 0}
            onPick={onPick}
            onClose={closeAll}
          />
        </Suspense>
      )}
    </div>
  )
})
