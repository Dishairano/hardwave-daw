import { memo, useCallback, useEffect, useState } from 'react'
import { MasterStrip } from './MasterStrip'
import { ChannelStrip } from './ChannelStrip'
import { useTrackStore } from '../../../stores/trackStore'
import { useMeterStore } from '../../../stores/meterStore'
import '../../primitives/Knob.css'
import '../../primitives/Fader.css'
import '../../primitives/Meter.css'
import './mixer-v2.css'

/**
 * Phase 1 of the FL Studio Wide 2 mixer redesign.
 *
 * Layout: master pinned left · scrollable insert column · empty 360 px FX
 * rack placeholder right. No virtualizer yet (Phase 4), no canvas meter
 * yet (Phase 4), no FX rack content yet (Phase 3), no plug-in picker yet
 * (Phase 3). What we ARE shipping in P1:
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
  // Master is special-cased; the insert column shows everything else that's
  // a mixable kind (audio / midi / bus / return).
  const tracks = useTrackStore((s) => s.tracks)
  const inserts = tracks.filter((t) => t.kind !== 'Master' && t.kind !== 'Automation')

  // Start listening to audio-thread meter events. Idempotent — meterStore
  // itself guards against double-subscribing.
  const startListening = useMeterStore((s) => s.startListening)
  useEffect(() => {
    startListening()
  }, [startListening])

  // Master starts selected so the FX rack panel has a sensible default
  // target on first open.
  const masterId = tracks.find((t) => t.kind === 'Master')?.id ?? null
  const [selectedId, setSelectedId] = useState<string | null>(masterId)
  // If the master id changes (project load) and we still pointed at the
  // old one, snap to the new one.
  useEffect(() => {
    if (!selectedId && masterId) setSelectedId(masterId)
  }, [masterId, selectedId])

  const onSelect = useCallback((id: string) => setSelectedId(id), [])

  return (
    <div className="mx-v2-root">
      <div className="mx-v2-body">
        <MasterStrip selected={selectedId === masterId} onSelect={onSelect} />
        <div className="mx-strips-wrap">
          <div className="mx-strips-scroll" id="mx-strips-scroll">
            <div className="mx-strips" id="mx-strips">
              {inserts.map((t, i) => (
                <ChannelStrip
                  key={t.id}
                  trackId={t.id}
                  index={i + 1}
                  selected={selectedId === t.id}
                  onSelect={onSelect}
                  variant="insert"
                  separator={t.kind === 'Bus' || t.kind === 'Return' ? 'group' : 'none'}
                />
              ))}
            </div>
          </div>
        </div>
        <div className="mx-fx-rack-placeholder" aria-label="FX rack (Phase 3)">
          <div className="mx-fx-rack-stub">
            <div className="mx-fx-rack-stub-title">FX RACK</div>
            <div className="mx-fx-rack-stub-sub">Phase 3 — selected: {selectedId ?? 'none'}</div>
          </div>
        </div>
      </div>
    </div>
  )
})
