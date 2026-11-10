import { useTrackStore } from '../../../stores/trackStore'
import { useNotificationStore } from '../../../stores/notificationStore'
import { memo, Suspense, lazy, useCallback, useState } from 'react'
import { useMixerSelectionStore } from '../../../stores/mixerSelectionStore'
import { useTrackById } from '../../../stores/trackStore'
import { usePluginFavoritesStore } from '../../../stores/pluginFavoritesStore'
import { usePluginCatalogStore } from '../../../stores/pluginCatalogStore'
import { RoutingMatrix } from './RoutingMatrix'
import { PlaysInto } from './PlaysInto'
import { FxSlot } from './FxSlot'
import { PluginPickerFlyout } from '../../plugin-picker/PluginPickerFlyout'
import { invoke } from '@tauri-apps/api/core'
import { t, tf, useLanguage } from '../../../i18n'
import { insertName } from '../../../lib/mixerNumbers'
import { withNewInsert } from '../../../lib/insertActions'

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
  useLanguage()
  const selectedId = useMixerSelectionStore((s) => s.selectedTrackId)
  const emptyNumber = useMixerSelectionStore((s) => s.selectedEmptyNumber)
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
      const failed = (e: unknown) => {
        console.error('add_plugin_to_track failed', e)
        useNotificationStore.getState().push('error', tf('The plug-in did not load: {error}', { error: String(e) }))
      }
      // An effect put on an empty number makes the insert, then loads the
      // effect on it: one undo step, and the new strip is selected.
      if (!track && emptyNumber != null) {
        usePluginFavoritesStore.getState().markUsed(pluginId)
        closeAll()
        void withNewInsert(emptyNumber, `Add an effect to insert ${emptyNumber}`, async (id) => {
          useMixerSelectionStore.getState().selectTrack(id)
          await invoke('add_plugin_to_track', { trackId: id, pluginId })
          await useTrackStore.getState().refreshTrack(id)
        }).catch(failed)
        return
      }
      if (!track) {
        closeAll()
        return
      }
      // Close at once; the slot shows when the plug-in has loaded. Nothing
      // polled for it before, so it only appeared once another window
      // reloaded the track list. The backend also tells other windows.
      usePluginFavoritesStore.getState().markUsed(pluginId)
      const targetTrackId = track.id
      closeAll()
      invoke('add_plugin_to_track', { trackId: targetTrackId, pluginId })
        .then(() => useTrackStore.getState().refreshTrack(targetTrackId))
        .catch(failed)
    },
    [track, emptyNumber, closeAll],
  )

  const picker = (
    <>
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
    </>
  )

  // An empty insert number: no track behind it yet. Its slots can take an
  // effect, which makes the insert.
  if (!track && emptyNumber != null) {
    return (
      <div className="mx-fx-rack">
        <div className="mx-fx-head">
          <div className="mx-fx-head-target">
            <div className="mx-fx-head-num">{emptyNumber}</div>
            <div>
              <div className="mx-fx-head-name">{insertName(emptyNumber)}</div>
              <div className="mx-fx-head-sub">{t('Empty insert · nothing plays into it')}</div>
            </div>
          </div>
        </div>
        <div className="mx-fx-routing">
          <div className="mx-fx-routing-head">
            <h4>{t('Routing')}</h4>
          </div>
          <div className="mx-fx-routing-empty">
            {t('No track is behind this number yet. Route a channel to it from the channel rack or the Output list, or put an effect on it below, and it becomes an insert.')}
          </div>
        </div>
        <div className="mx-fx-slots">
          <h4>
            {t('FX Slots')} <span>{tf('{n} of {total} · {active} active', { n: SLOT_COUNT, total: SLOT_COUNT, active: 0 })}</span>
          </h4>
          {Array.from({ length: SLOT_COUNT }, (_, i) => (
            <FxSlot key={`empty-${i}`} trackId="" slotIndex={i} onOpenPicker={onOpenPicker} />
          ))}
        </div>
        {picker}
      </div>
    )
  }

  if (!track) {
    return (
      <div className="mx-fx-rack">
        <div className="mx-fx-rack-empty">{t('No track selected')}</div>
      </div>
    )
  }

  // The badge is the insert's mixer number, M for the master. It guessed a
  // number from digits in the name before ("808 Kick" read 808).
  const isMaster = track.kind === 'Master'
  const badge = isMaster ? 'M' : (track.mixerNumber != null ? String(track.mixerNumber) : '·')
  // "Insert 7 · into the master", as the mockup's panel says where it goes.
  const outTo = useTrackStore.getState().tracksById[track.outputBus ?? '']
  const into = outTo && outTo.kind !== 'Master'
    ? tf('into {name}', { name: outTo.mixerNumber != null ? `${outTo.mixerNumber} ${insertName(outTo.mixerNumber, outTo)}` : outTo.name })
    : t('into the master')
  const roleLine = isMaster
    ? t('Master · Stereo · final output')
    : track.mixerNumber != null
      ? `${tf('Insert {n}', { n: track.mixerNumber })}${track.muted ? ` · ${t('muted')}` : ''} · ${into}`
      : track.muted ? tf('{kind} · muted', { kind: track.kind }) : tf('{kind} · active', { kind: track.kind })

  // Pad inserts up to SLOT_COUNT so the row count is always 10.
  const insertsPadded = Array.from({ length: SLOT_COUNT }, (_, i) => track.inserts[i])
  const activeCount = track.inserts.filter((s) => s.enabled).length

  return (
    <div className="mx-fx-rack">
      <div className="mx-fx-head">
        <div className="mx-fx-head-target">
          <div className="mx-fx-head-num">{badge}</div>
          <div>
            <div className="mx-fx-head-name">{isMaster ? track.name : insertName(track.mixerNumber ?? -1, track)}</div>
            <div className="mx-fx-head-sub">{roleLine}</div>
          </div>
        </div>
      </div>

      <RoutingMatrix trackId={track.id} />
      {track.mixerNumber != null && <PlaysInto trackId={track.id} />}

      <div className="mx-fx-slots">
        <h4>
          {t('FX Slots')} <span>{tf('{n} of {total} · {active} active', { n: SLOT_COUNT, total: SLOT_COUNT, active: activeCount })}</span>
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

      {picker}
    </div>
  )
})
