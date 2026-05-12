import { memo, useCallback, useEffect } from 'react'
import { MasterStrip } from './MasterStrip'
import { StripsScroller } from './StripsScroller'
import { FxRackPanel } from './FxRackPanel'
import { useTrackStore } from '../../../stores/trackStore'
import { useMeterStore } from '../../../stores/meterStore'
import { useSendStore } from '../../../stores/sendStore'
import { useMixerSelectionStore } from '../../../stores/mixerSelectionStore'
import '../../primitives/Knob.css'
import '../../primitives/Fader.css'
import '../../primitives/Meter.css'
import './mixer-v2.css'

/**
 * Phase 1 of the FL Studio Wide 2 mixer redesign.
 *
 * Layout: master pinned left · scrollable insert column · 360 px FX rack
 * right. No virtualizer yet (Phase 4), no canvas meter yet (Phase 4).
 * Phase 3 (this commit) wires the FX rack + plug-in picker. What we ARE
 * shipping at this point:
 *  - the new visual shape exactly matching the approved mockup
 *  - the Knob / Fader / Meter primitives
 *  - selection state (click any strip retargets the future FX rack)
 *  - fine-grained Zustand subscriptions per strip
 *  - optimistic-local fader/pan drag → single commit on pointerup
 *
 * Wrapped in the `experimentalMixer` feature flag — the legacy MixerPanel
 * stays the default until phase 3 is done.
 */
export const MixerPanelV2 = memo(function MixerPanelV2() {
  // Track list — only needed for the master id lookup. The inserts column
  // (StripsScroller) subscribes itself.
  const tracks = useTrackStore((s) => s.tracks)

  // Start listening to audio-thread meter events. Idempotent — meterStore
  // itself guards against double-subscribing.
  const startListening = useMeterStore((s) => s.startListening)
  useEffect(() => {
    startListening()
  }, [startListening])

  // The FX rack's RoutingMatrix reads from sendStore.byTrack — but only
  // the Settings page used to call fetchAll(). Fire it once on mount so
  // the rack has data the moment the user clicks a strip. Subsequent
  // edits (addSend / setPreFader / etc) refetch internally.
  const fetchAllSends = useSendStore((s) => s.fetchAll)
  useEffect(() => {
    fetchAllSends().catch((e) => console.error('initial sends fetch failed', e))
  }, [fetchAllSends])

  // Selection lives in its own store — see mixerSelectionStore for why.
  const masterId = tracks.find((t) => t.kind === 'Master')?.id ?? null
  const selectedId = useMixerSelectionStore((s) => s.selectedTrackId)
  const selectTrack = useMixerSelectionStore((s) => s.selectTrack)
  // Seed the selection with master on first mount + on project load so
  // the FX rack always has a sensible target.
  useEffect(() => {
    if (!selectedId && masterId) selectTrack(masterId)
  }, [masterId, selectedId, selectTrack])

  const onSelect = useCallback((id: string) => selectTrack(id), [selectTrack])

  return (
    <div className="mx-v2-root">
      <div className="mx-v2-body">
        <MasterStrip selected={selectedId === masterId} onSelect={onSelect} />
        <StripsScroller selectedId={selectedId} onSelect={onSelect} />
        <FxRackPanel />
      </div>
    </div>
  )
})
