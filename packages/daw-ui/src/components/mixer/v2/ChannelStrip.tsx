import { memo, useCallback, useMemo } from 'react'
import { useSendStore, type SendInfo } from '../../../stores/sendStore'
import { usePerfMetersStore } from '../../../stores/perfMetersStore'
import { Knob } from '../../primitives/Knob'
import { Fader } from '../../primitives/Fader'
import { MeterPair } from '../../primitives/Meter'
import { DbScale } from './DbScale'
import { SendsDots } from './SendsDots'
import {
  useTrackName,
  useTrackVolume,
  useTrackPan,
  useTrackStereoSeparation,
  useTrackStore,
  useTrackKind,
  useTrackColor,
  useTrackMuted,
  useTrackSoloed,
  useTrackArmed,
} from '../../../stores/trackStore'
import { useMixerSettingsStore } from '../../../stores/mixerSettingsStore'
import { useMeterStore } from '../../../stores/meterStore'
import {
  normalizePan,
  normalizeVolumeDb,
  useAutomationWriteStore,
} from '../../../stores/automationWriteStore'


const NO_SENDS: SendInfo[] = []
export interface ChannelStripProps {
  trackId: string
  /** Numeric position (1-based) shown in the s-num header. Master uses 'M'. */
  index: number | string
  /** Selected = highlighted + FX rack targets this strip. */
  selected: boolean
  onSelect: (trackId: string) => void
  /** Visual variant — master gets the red theme. */
  variant?: 'insert' | 'master'
  /** Group separator at the left edge (auto for buses/returns). */
  separator?: 'none' | 'group' | 'user'
}

/**
 * One mixer channel strip. All visual primitives composed here; data
 * subscriptions are fine-grained so a fader drag on strip 3 only re-renders
 * strip 3 — not the other 499 strips.
 *
 * Wiring summary:
 * - volume_db → Fader, drag uses optimistic-local + single commitVolume
 *   on pointerup. Wheel debounces commit.
 * - pan       → Pan Knob, same optimistic-local + commitPan model.
 * - width     → Width Knob, maps -100..+100 onto stereoSeparation 0..2 via
 *   set_track_stereo_separation. Phase 1 wires direct commit (no local
 *   pattern) since wheel-on-knob is the dominant input and already
 *   debounced inside the Knob primitive.
 * - meter L/R → useTrackMeter(id), per-strip subscription via Zustand
 *   selector. Phase 4 replaces this with a canvas painted from a single
 *   global rAF loop.
 */
export const ChannelStrip = memo(function ChannelStrip(props: ChannelStripProps) {
  const { trackId, index, selected, onSelect, variant = 'insert', separator = 'none' } = props

  const name = useTrackName(trackId)
  const kind = useTrackKind(trackId)
  const color = useTrackColor(trackId)
  const setVolumeDb = useTrackVolume(trackId)
  const setPan = useTrackPan(trackId)
  // With automation on the fader or pan they move with the song, as in FL,
  // except while this strip is recording a move of its own.
  const autoVolumeDb = useMeterStore((s) => s.tracks[trackId]?.autoVolumeDb ?? null)
  const autoPan = useMeterStore((s) => s.tracks[trackId]?.autoPan ?? null)
  const writingVol = useAutomationWriteStore((s) => s.touching.some((k) => k === `${trackId}:{"kind":"track_volume"}`))
  const writingPan = useAutomationWriteStore((s) => s.touching.some((k) => k === `${trackId}:{"kind":"track_pan"}`))
  const volumeDb = autoVolumeDb != null && !writingVol ? Math.max(-60, autoVolumeDb) : setVolumeDb
  const pan = autoPan != null && !writingPan ? autoPan : setPan
  const stereoSep = useTrackStereoSeparation(trackId)
  const muted = useTrackMuted(trackId)
  const soloed = useTrackSoloed(trackId)
  const armed = useTrackArmed(trackId)
  const showWidthKnob = useMixerSettingsStore((s) => s.showWidthKnob)
  // Where this channel goes, and its sends as the dots under it. The dots
  // were a fixed pattern before: every strip showed one send it did not have.
  const outputName = useTrackStore((s) => {
    const bus = s.tracksById[trackId]?.outputBus
    return bus ? (s.tracksById[bus]?.name ?? null) : null
  })
  const sends = useSendStore((s) => s.byTrack[trackId] ?? NO_SENDS)
  const sendDots = useMemo(
    () => sends.slice(0, 5).map((x) => (x.enabled ? (x.preFader ? 'pre' : 'post') : false) as false | 'pre' | 'post'),
    [sends],
  )

  // ---- volume ----
  // Each drag also streams into automation when a write mode is on. The
  // recorder existed with no caller, so a fader move during playback wrote
  // nothing; these two lines are where a move becomes a lane.
  const onVolChange = useCallback(
    (db: number) => {
      useTrackStore.getState().setVolumeLocal(trackId, db)
      // Heard while dragging, not only on release.
      useTrackStore.getState().sendLiveMix(trackId)
      useAutomationWriteStore
        .getState()
        .writeSample(trackId, { kind: 'track_volume' }, normalizeVolumeDb(db))
    },
    [trackId],
  )
  const onVolCommit = useCallback(
    (db: number) => {
      useTrackStore.getState().commitVolume(trackId, db).catch(console.error)
      useAutomationWriteStore.getState().endTouch(trackId, { kind: 'track_volume' })
    },
    [trackId],
  )

  // ---- pan ----
  const onPanChange = useCallback(
    (next: number) => {
      useTrackStore.getState().setPanLocal(trackId, next)
      useTrackStore.getState().sendLiveMix(trackId)
      useAutomationWriteStore
        .getState()
        .writeSample(trackId, { kind: 'track_pan' }, normalizePan(next))
    },
    [trackId],
  )
  const onPanCommit = useCallback(
    (next: number) => {
      useTrackStore.getState().commitPan(trackId, next).catch(console.error)
      useAutomationWriteStore.getState().endTouch(trackId, { kind: 'track_pan' })
    },
    [trackId],
  )

  // ---- width (mapped to stereoSeparation 0..2 backend command) ----
  const onWidthChange = useCallback(
    (next: number) => {
      // Heard while turning; one undo step when the turn ends.
      useTrackStore.getState().setWidthLocal(trackId, next)
      useTrackStore.getState().sendLiveMix(trackId)
    },
    [trackId],
  )
  const onWidthCommit = useCallback(
    (next: number) => {
      useTrackStore.getState().setTrackStereoSeparation(trackId, next).catch(console.error)
    },
    [trackId],
  )

  const onClick = useCallback(() => onSelect(trackId), [trackId, onSelect])

  // ---- M/S/R pill handlers — stopPropagation so clicking the pill
  // doesn't also re-select the strip ----
  const onMute = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      useTrackStore.getState().toggleMute(trackId).catch(console.error)
    },
    [trackId],
  )
  const load = usePerfMetersStore(s => s.trackLoad[trackId] ?? 0)

  const onSolo = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      // Ctrl-click solos this strip alone; a plain click adds to the set.
      const store = useTrackStore.getState()
      const action = e.ctrlKey || e.metaKey
        ? store.soloExclusively(trackId)
        : store.toggleSolo(trackId)
      action.catch(console.error)
    },
    [trackId],
  )
  const onArm = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      useTrackStore.getState().toggleArm(trackId).catch(console.error)
    },
    [trackId],
  )

  const isAudioOrMidi = kind === 'Audio' || kind === 'Midi'

  // Color-tag class derived from the track's color field if present, else
  // from kind. Master variant overrides everything.
  const colorClass =
    variant === 'master'
      ? 'master'
      : color
        ? 'color-' + color.toLowerCase()
        : kind === 'Bus'
          ? 'color-bus'
          : kind === 'Return'
            ? 'color-rev'
            : ''

  const sepClass =
    separator === 'group' ? ' sep-left' : separator === 'user' ? ' sep-user' : ''

  return (
    <div
      className={'mx-strip ' + colorClass + (selected ? ' selected' : '') + sepClass}
      onClick={onClick}
      data-track-id={trackId}
      data-idx={index}
    >
      <div className="mx-s-num">
        <span className="mx-s-num-idx">{index}</span>
        {variant !== 'master' && (
          <div className="mx-s-pills">
            <button
              className={'mx-pill mx-pill-m' + (muted ? ' on' : '')}
              onClick={onMute}
              title={muted ? 'Unmute' : 'Mute'}
            >
              M
            </button>
            <button
              className={'mx-pill mx-pill-s' + (soloed ? ' on' : '')}
              onClick={onSolo}
              title={`${soloed ? 'Unsolo' : 'Solo'}. Ctrl-click: solo this one alone`}
            >
              S
            </button>
            {isAudioOrMidi && (
              <button
                className={'mx-pill mx-pill-r' + (armed ? ' on' : '')}
                onClick={onArm}
                title={armed ? 'Disarm' : 'Arm for recording'}
              >
                R
              </button>
            )}
          </div>
        )}
      </div>
      <div className="mx-s-name">{name || 'Track'}</div>
      {/* What share of each audio block this track is taking. The CPU meter
          in the toolbar says the load is high; this says which track. Shown
          only once a track is worth noticing, so a mixer full of quiet
          strips is not a wall of 0%. */}
      {load >= 3 && (
        <div
          title={`This track is using ${load.toFixed(0)}% of each audio block`}
          style={{
            fontSize: 8, textAlign: 'center', letterSpacing: 0.3,
            color: load >= 40 ? '#EF4444' : load >= 15 ? '#F59E0B' : 'var(--text-dim)',
          }}
        >
          {load.toFixed(0)}%
        </div>
      )}

      {/* The master has no pan or width of its own: the knobs did nothing
          there, so they are not shown. The row keeps its height so the
          faders still line up. */}
      <div className="mx-s-knob-row" style={kind === 'Master' ? { visibility: 'hidden' } : undefined}>
        <div className="mx-knob-cell">
          <Knob
            value={pan}
            min={-1}
            max={1}
            defaultValue={0}
            kind="pan"
            onChange={onPanChange}
            onChangeEnd={onPanCommit}
            title="Pan"
          />
          <span className="mx-klabel">PAN</span>
        </div>
        {showWidthKnob && (
          <div className="mx-knob-cell">
            <Knob
              value={stereoSep}
              min={0}
              max={2}
              defaultValue={1}
              kind="width"
              onChange={onWidthChange}
              onChangeEnd={onWidthCommit}
              title="Width"
            />
            <span className="mx-klabel">WIDTH</span>
          </div>
        )}
      </div>

      <div className="mx-s-fader-meter">
        <div className="mx-fader-col">
          <Fader
            valueDb={volumeDb}
            onChange={onVolChange}
            onChangeEnd={onVolCommit}
            title="Volume"
          />
        </div>
        <DbScale />
        <MeterPair trackId={trackId} />
      </div>

      <div className="mx-s-db">
        {volumeDb <= -60 ? '-∞' : volumeDb.toFixed(1)} dB
      </div>

      {kind !== 'Master' && (
        <div
          className={'mx-s-out' + (outputName ? ' routed' : '')}
          title={`Goes to ${outputName ?? 'Master'}. Change it under Output in the FX rack.`}
        >
          → {outputName ?? 'Master'}
        </div>
      )}
      <SendsDots sends={sendDots} />
    </div>
  )
})
