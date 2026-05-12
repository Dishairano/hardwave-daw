import { memo, useCallback } from 'react'
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
  const volumeDb = useTrackVolume(trackId)
  const pan = useTrackPan(trackId)
  const stereoSep = useTrackStereoSeparation(trackId)
  const muted = useTrackMuted(trackId)
  const soloed = useTrackSoloed(trackId)
  const armed = useTrackArmed(trackId)
  const showWidthKnob = useMixerSettingsStore((s) => s.showWidthKnob)

  // ---- volume ----
  const onVolChange = useCallback(
    (db: number) => useTrackStore.getState().setVolumeLocal(trackId, db),
    [trackId],
  )
  const onVolCommit = useCallback(
    (db: number) => {
      useTrackStore.getState().commitVolume(trackId, db).catch(console.error)
    },
    [trackId],
  )

  // ---- pan ----
  const onPanChange = useCallback(
    (next: number) => useTrackStore.getState().setPanLocal(trackId, next),
    [trackId],
  )
  const onPanCommit = useCallback(
    (next: number) => {
      useTrackStore.getState().commitPan(trackId, next).catch(console.error)
    },
    [trackId],
  )

  // ---- width (mapped to stereoSeparation 0..2 backend command) ----
  const onWidthChange = useCallback(
    (next: number) => {
      // Knob value range is the underlying separation (0..2). Direct apply
      // — keep the gesture snappy. Tauri command does its own clamp.
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
  const onSolo = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      useTrackStore.getState().toggleSolo(trackId).catch(console.error)
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
              title={soloed ? 'Unsolo' : 'Solo'}
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

      <div className="mx-s-knob-row">
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

      <SendsDots sends={[false, 'post', false, false, false]} />
    </div>
  )
})
