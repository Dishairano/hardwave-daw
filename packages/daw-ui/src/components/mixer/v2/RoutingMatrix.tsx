import { memo, useCallback, useMemo, useState } from 'react'
import { useSendStore, type SendInfo } from '../../../stores/sendStore'
import { useTrackStore, type TrackInfo } from '../../../stores/trackStore'
import { useNotificationStore } from '../../../stores/notificationStore'

export interface RoutingMatrixProps {
  trackId: string
}

// Module-level frozen empty array — keeps the Zustand selector's
// identity stable when a track has no sends, so the component doesn't
// render-loop.
const EMPTY_SENDS: SendInfo[] = []

/// A new project's inserts are called "Insert 12" and so on. Those are
/// listed after the channels someone named or put something on.
const DEFAULT_NAME = /^Insert \d+$/

/**
 * Routing for the selected channel, at the top of the FX rack:
 *
 *   Output   [Master ▾]           where this channel's sound goes
 *   Sends    ▶ Reverb  [----|--] -6.0 dB  post  ×
 *            [+ Send to… ▾]
 *
 * Output moves the whole channel (FL's "route to this track only");
 * a send copies some of it to another channel on top of that.
 */
export const RoutingMatrix = memo(function RoutingMatrix({ trackId }: RoutingMatrixProps) {
  const sends = useSendStore((s) => s.byTrack[trackId] ?? EMPTY_SENDS)
  const tracks = useTrackStore((s) => s.tracks)
  const self = tracks.find((t) => t.id === trackId)
  const isMaster = self?.kind === 'Master'
  // Send levels while a slider is held, so the number follows the hand
  // and the store is written once on release.
  const [dragging, setDragging] = useState<Record<number, number>>({})

  const { used, empty } = useMemo(() => {
    const used: TrackInfo[] = []
    const empty: TrackInfo[] = []
    for (const t of tracks) {
      if (t.id === trackId || t.kind === 'Master') continue
      const inUse =
        !DEFAULT_NAME.test(t.name) || t.inserts.length > 0 || t.id === self?.outputBus
      ;(inUse ? used : empty).push(t)
    }
    return { used, empty }
  }, [tracks, trackId, self?.outputBus])

  // A refused route (a loop back to this channel, say) says why on screen.
  const report = (what: string) => (e: unknown) => {
    console.error(what, e)
    useNotificationStore.getState().push('error', `${what}: ${String(e)}`)
  }

  const setOutput = useCallback(
    (value: string) => {
      useTrackStore
        .getState()
        .setTrackOutputBus(trackId, value === '' ? null : value)
        .catch(report('Could not change the output'))
    },
    [trackId],
  )

  const addSend = useCallback(
    (target: string) => {
      if (!target) return
      useSendStore.getState().addSend(trackId, target, -6, false).catch(report('Could not add the send'))
    },
    [trackId],
  )

  const destinations = (exclude: Set<string>) => (
    <>
      {used.length > 0 && (
        <optgroup label="Channels in use">
          {used.filter((t) => !exclude.has(t.id)).map((t) => (
            <option key={t.id} value={t.id}>{t.name}</option>
          ))}
        </optgroup>
      )}
      <optgroup label="Empty inserts">
        {empty.filter((t) => !exclude.has(t.id)).map((t) => (
          <option key={t.id} value={t.id}>{t.name}</option>
        ))}
      </optgroup>
    </>
  )

  if (isMaster) {
    return (
      <div className="mx-fx-routing">
        <div className="mx-fx-routing-head">
          <h4>Routing</h4>
        </div>
        <div className="mx-fx-routing-empty">The master goes to your audio output.</div>
      </div>
    )
  }

  const sentTo = new Set(sends.map((s) => s.target))

  return (
    <div className="mx-fx-routing">
      <div className="mx-fx-routing-head">
        <h4>Output</h4>
      </div>
      <select
        className="mx-fx-route-select"
        value={self?.outputBus ?? ''}
        onChange={(e) => setOutput(e.target.value)}
        title="Where this channel's sound goes"
      >
        <option value="">Master</option>
        {destinations(new Set())}
      </select>

      <div className="mx-fx-routing-head" style={{ marginTop: 10 }}>
        <h4>Sends</h4>
      </div>
      {sends.length === 0 && (
        <div className="mx-fx-routing-empty">
          No sends. A send copies part of this channel to another one, for a shared reverb or delay.
        </div>
      )}
      <div className="mx-fx-route-list">
        {sends.map((send) => {
          const dest = tracks.find((t) => t.id === send.target)?.name ?? '(missing channel)'
          const active = send.enabled
          const rev = send.preFader
          const rowCls =
            'mx-fx-route' + (active ? ' on' : '') + (active && rev ? ' rev' : '')
          const gain = dragging[send.index] ?? send.gainDb
          return (
            <div key={send.index} className={rowCls}>
              <button
                className="mx-fx-route-arr"
                onClick={() =>
                  useSendStore
                    .getState()
                    .setEnabled(trackId, send.index, !active)
                    .catch(report('Could not switch the send'))
                }
                title={active ? 'Turn this send off' : 'Turn this send on'}
                aria-label={active ? 'Turn send off' : 'Turn send on'}
              />
              <div className="mx-fx-route-dest" title={dest}>{dest}</div>
              <input
                className="mx-fx-route-gain"
                type="range"
                min={-60}
                max={6}
                step={0.1}
                value={gain}
                onChange={(e) =>
                  setDragging((d) => ({ ...d, [send.index]: Number(e.target.value) }))
                }
                onPointerUp={(e) => {
                  const v = Number((e.target as HTMLInputElement).value)
                  setDragging((d) => {
                    const { [send.index]: _, ...rest } = d
                    return rest
                  })
                  useSendStore.getState().setGain(trackId, send.index, v).catch(report('Could not set the send level'))
                }}
                onDoubleClick={() =>
                  useSendStore.getState().setGain(trackId, send.index, 0).catch(report('Could not set the send level'))
                }
                title="Send level (double-click for 0 dB)"
              />
              <div className="mx-fx-route-amt">{gain <= -60 ? '-∞' : gain.toFixed(1)} dB</div>
              <button
                className="mx-fx-route-pp"
                onClick={() =>
                  useSendStore
                    .getState()
                    .setPreFader(trackId, send.index, !rev)
                    .catch(report('Could not switch the send'))
                }
                title={rev ? 'Before the fader. Click: after the fader' : 'After the fader. Click: before the fader'}
              >
                {rev ? 'pre' : 'post'}
              </button>
              <button
                className="mx-fx-route-x"
                onClick={() =>
                  useSendStore.getState().removeSend(trackId, send.index).catch(report('Could not remove the send'))
                }
                title="Remove this send"
                aria-label="Remove send"
              >
                ×
              </button>
            </div>
          )
        })}
      </div>
      <select
        className="mx-fx-route-select"
        value=""
        onChange={(e) => addSend(e.target.value)}
        title="Copy part of this channel to another channel"
        style={{ marginTop: 6 }}
      >
        <option value="">+ Send to…</option>
        {destinations(sentTo)}
      </select>
    </div>
  )
})
