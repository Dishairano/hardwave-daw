import { memo, useCallback } from 'react'
import { useSendStore, type SendInfo } from '../../../stores/sendStore'
import { useTrackStore } from '../../../stores/trackStore'

export interface RoutingMatrixProps {
  trackId: string
}

// Module-level frozen empty array — keeps the Zustand selector's
// identity stable when a track has no sends, so the component doesn't
// render-loop.
const EMPTY_SENDS: SendInfo[] = []

/**
 * "Send Routing" block at the top of the FX rack. Shows the selected
 * track's sends as a vertical list:
 *
 *   ▶  Destination Name              -6.0 dB    post
 *
 * Visuals follow the mockup palette (.route / .route.on / .route.on.rev):
 *   - inactive  → dim arrow, "—" amount, "off" pill
 *   - post-fader → green arrow, dB amount, "post" pill
 *   - pre-fader  → blue arrow, dB amount, "pre" pill
 *
 * Phase 3 ships READ + pre/post toggle only. Drag-to-add new sends, gain
 * sliders, and inline rename land in Phase 5 — the spec explicitly defers
 * those. Per-row click on the pre/post pill flips that send and refetches.
 *
 * Data source: `sendStore.byTrack[trackId]` (confirmed shape: SendInfo[]
 * with fields `index`, `target`, `gainDb`, `preFader`, `enabled` — see
 * stores/sendStore.ts:13). Track names looked up against trackStore.
 */
export const RoutingMatrix = memo(function RoutingMatrix({ trackId }: RoutingMatrixProps) {
  // Use the cached send list for this track. The mixer-level fetchAll
  // happens once on mixer mount + on add/remove/edit — we deliberately
  // do NOT fire a fetch in this component to avoid the FX rack
  // re-fetching on every selection change.
  const sends = useSendStore((s) => s.byTrack[trackId] ?? EMPTY_SENDS)
  // Subscribe to the raw tracks array — Zustand re-runs the selector on
  // every state set, but the consumer only re-renders when the returned
  // reference changes. Tracks is replaced wholesale on every fetchTracks,
  // so this re-renders on track edits but stays cheap (route list is
  // tiny). Name lookup is a `.find` per row — fine for <500 tracks.
  const tracks = useTrackStore((s) => s.tracks)

  const togglePrePost = useCallback(
    (send: SendInfo) => {
      // Single IPC; sendStore refetches once internally.
      useSendStore
        .getState()
        .setPreFader(trackId, send.index, !send.preFader)
        .catch((e) => console.error('setPreFader failed', e))
    },
    [trackId],
  )

  if (sends.length === 0) {
    return (
      <div className="mx-fx-routing">
        <div className="mx-fx-routing-head">
          <h4>Send Routing</h4>
        </div>
        <div className="mx-fx-routing-empty">No sends on this track</div>
      </div>
    )
  }

  return (
    <div className="mx-fx-routing">
      <div className="mx-fx-routing-head">
        <h4>Send Routing</h4>
      </div>
      <div className="mx-fx-route-list">
        {sends.map((send) => {
          const dest = tracks.find((t) => t.id === send.target)?.name ?? '(missing track)'
          const active = send.enabled
          const rev = send.preFader
          const rowCls =
            'mx-fx-route' + (active ? ' on' : '') + (active && rev ? ' rev' : '')
          const amt = active ? `${send.gainDb.toFixed(1)} dB` : '—'
          const pp = active ? (rev ? 'pre' : 'post') : 'off'
          return (
            <div key={send.index} className={rowCls}>
              <div className="mx-fx-route-arr" aria-hidden="true" />
              <div className="mx-fx-route-dest">{dest}</div>
              <div className="mx-fx-route-amt">{amt}</div>
              <button
                className="mx-fx-route-pp"
                onClick={() => togglePrePost(send)}
                title={rev ? 'Switch to post-fader' : 'Switch to pre-fader'}
              >
                {pp}
              </button>
            </div>
          )
        })}
      </div>
    </div>
  )
})

