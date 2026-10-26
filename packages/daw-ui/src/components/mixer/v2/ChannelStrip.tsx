import { MasterCorrelation } from './MasterCorrelation'
import { memo, useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
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
  monitorModeOf,
  nextMonitorMode,
  type MonitorMode,
} from '../../../stores/trackStore'
import { useMixerSettingsStore } from '../../../stores/mixerSettingsStore'
import { useMeterStore } from '../../../stores/meterStore'
import { useVcaStore } from '../../../stores/vcaStore'
import {
  normalizePan,
  normalizeVolumeDb,
  useAutomationWriteStore,
} from '../../../stores/automationWriteStore'
import { t, tf, useLanguage } from '../../../i18n'


const NO_SENDS: SendInfo[] = []

const MONITOR_TITLES: Record<MonitorMode, string> = {
  off: 'Monitoring off: the input is recorded, not heard. Click for In',
  in: 'Monitoring In: the input is always heard while armed. Click for Auto',
  auto: 'Monitoring Auto: the input while stopped or recording, this track while playing back. Click to switch off',
}
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
/** The highest peak a strip has reached since it was last clicked, in dB.
 *  The engine sends the level of every strip; only the old mixer kept a clip
 *  light and the default one showed bare meters. Read without re-rendering
 *  the strip on every meter tick: it only renders when a new highest comes. */
function useHighestPeak(trackId: string, master: boolean): [number, () => void] {
  const [highest, setHighest] = useState(-100)
  const held = useRef(-100)
  useEffect(() => useMeterStore.subscribe((s) => {
    const m = s.tracks[trackId]
    const db = master ? s.master.peak_db : Math.max(m?.peakL ?? -100, m?.peakR ?? -100)
    if (db > held.current + 0.05) {
      held.current = db
      setHighest(db)
    }
  }), [trackId, master])
  const reset = useCallback(() => {
    held.current = -100
    setHighest(-100)
  }, [])
  return [highest, reset]
}

function LockIcon() {
  return (
    <svg width="9" height="10" viewBox="0 0 9 10" aria-hidden="true">
      <rect x="1" y="4.5" width="7" height="5" rx="1" fill="currentColor" />
      <path d="M2.5 4.5V3a2 2 0 0 1 4 0v1.5" fill="none" stroke="currentColor" strokeWidth="1.2" />
    </svg>
  )
}

export const ChannelStrip = memo(function ChannelStrip(props: ChannelStripProps) {
  useLanguage()
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
  const soloSafe = useTrackStore((s) => s.tracksById[trackId]?.solo_safe ?? false)
  const monitorMode = useTrackStore((s) => {
    const t = s.tracksById[trackId]
    return t ? monitorModeOf(t) : 'in'
  })
  const [highestPeak, resetPeak] = useHighestPeak(trackId, variant === 'master')
  // The VCAs this track is in, as amber tags on its cap.
  const vcaTagsKey = useVcaStore((s) =>
    s.vcas.flatMap((v, i) => (v.members.includes(trackId) ? [`V${i + 1}`] : [])).join(','))
  const vcaTags = useMemo(() => (vcaTagsKey ? vcaTagsKey.split(',') : []), [vcaTagsKey])
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
  // Hearing the input while armed, and solo-safe: both had a button only in
  // the old mixer and in the old track list's menu.
  const onMonitor = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      useTrackStore.getState().setTrackMonitorMode(trackId, nextMonitorMode(monitorMode)).catch(console.error)
    },
    [trackId, monitorMode],
  )
  const onSoloSafe = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      useTrackStore.getState().toggleSoloSafe(trackId).catch(console.error)
    },
    [trackId],
  )
  const onResetPeak = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      resetPeak()
    },
    [resetPeak],
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
      // The track's own colour, for the strip's cap. colorClass only knows
      // named tags, so a hex colour from the track never showed.
      style={color && /^#[0-9a-f]{6}$/i.test(color) ? ({ ['--strip-color' as string]: color } as CSSProperties) : undefined}
      onClick={onClick}
      data-track-id={trackId}
      data-idx={index}
    >
      <div className="mx-s-num">
        <span className="mx-s-num-idx">
          {index}
          {vcaTags.map(tag => <span key={tag} className="mx-vca-tag">{tag}</span>)}
        </span>
        {variant !== 'master' && (
          <div className="mx-s-pills">
            <button
              className={'mx-pill mx-pill-m' + (muted ? ' on' : '')}
              onClick={onMute}
              title={muted ? t('Unmute') : t('Mute')}
            >
              M
            </button>
            <button
              className={'mx-pill mx-pill-s' + (soloed ? ' on' : '')}
              onClick={onSolo}
              title={soloed ? t('Unsolo. Ctrl-click: solo this one alone') : t('Solo. Ctrl-click: solo this one alone')}
            >
              S
            </button>
            {isAudioOrMidi && (
              <button
                className={'mx-pill mx-pill-r' + (armed ? ' on' : '')}
                onClick={onArm}
                title={armed ? t('Disarm') : t('Arm for recording')}
              >
                R
              </button>
            )}
            {isAudioOrMidi && armed && (
              <button
                className={'mx-pill mx-pill-in' + (monitorMode !== 'off' ? ' on' : '')}
                onClick={onMonitor}
                title={t(MONITOR_TITLES[monitorMode])}
              >
                {monitorMode === 'auto' ? 'AUTO' : 'IN'}
              </button>
            )}
            <button
              className={'mx-pill mx-pill-safe' + (soloSafe ? ' on' : '')}
              onClick={onSoloSafe}
              title={soloSafe
                ? t('Solo-safe: soloing another track does not silence this one. Click to turn off')
                : t('Solo-safe: keep this track playing when another is soloed')}
            >
              <LockIcon />
            </button>
          </div>
        )}
      </div>
      <div className="mx-s-name">{name || t('Track')}</div>
      {/* What share of each audio block this track is taking. The CPU meter
          in the toolbar says the load is high; this says which track. Shown
          only once a track is worth noticing, so a mixer full of quiet
          strips is not a wall of 0%. */}
      {load >= 3 && (
        <div
          title={tf('This track is using {n}% of each audio block', { n: load.toFixed(0) })}
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
            title={t('Pan')}
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
              title={t('Width')}
            />
            <span className="mx-klabel">{t('WIDTH')}</span>
          </div>
        )}
      </div>

      {/* The highest peak since a click on it, red once it went over 0 dB:
          a clip that happened while you looked away stays visible. */}
      <button
        type="button"
        className={'mx-s-peak' + (highestPeak >= 0 ? ' clipped' : '')}
        onClick={onResetPeak}
        title={highestPeak > -100 ? t('Highest peak so far. Click to start again') : t('Highest peak')}
      >
        <span className="mx-s-clip-led" />
        <span className="mx-s-peak-db">
          {highestPeak <= -100 ? '-∞' : (highestPeak > 0 ? '+' : '') + highestPeak.toFixed(1)}
        </span>
      </button>

      <div className="mx-s-fader-meter">
        <div className="mx-fader-col">
          <Fader
            valueDb={volumeDb}
            onChange={onVolChange}
            onChangeEnd={onVolCommit}
            title={t('Volume')}
          />
        </div>
        <DbScale />
        <MeterPair trackId={trackId} />
      </div>

      <div className="mx-s-db">
        {volumeDb <= -60 ? '-∞' : volumeDb.toFixed(1)} dB
      </div>

      {kind === 'Master' && <MasterCorrelation />}
      {kind !== 'Master' && (
        <div
          className={'mx-s-out' + (outputName ? ' routed' : '')}
          title={tf('Goes to {name}. Change it under Output in the FX rack.', { name: outputName ?? 'Master' })}
        >
          → {outputName ?? 'Master'}
        </div>
      )}
      <SendsDots sends={sendDots} />
    </div>
  )
})
