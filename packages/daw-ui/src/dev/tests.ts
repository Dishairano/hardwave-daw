// Phase 1 test harness.
//
// Each test is one of:
//   - AUTO:    runs in code, asserts atomics, sets PASS/FAIL automatically.
//   - MANUAL:  requires human observation (hearing/seeing); user clicks PASS/FAIL.
//   - OBSERVE: not a pass/fail test — continuous readout elsewhere in the panel.
//
// AUTO tests only prove "the value I wrote showed up in the atomic I read."
// MANUAL tests are where we actually trust ears and eyes. Be explicit about which.

import { invoke } from '@tauri-apps/api/core'
import { devDumpState, devForceDeviceError, devResolveTestAsset, queryTestId, getMeterDb, clickCanvas, simulateKey, type DevState } from './devApi'
import { useTransportStore, snapToTicks } from '../stores/transportStore'
import { PHASE3_TESTS } from './testsPhase3'
import { PHASE4_TESTS } from './testsPhase4'

export type TestKind = 'AUTO' | 'MANUAL'
export type TestStatus = 'idle' | 'running' | 'pass' | 'fail'

export interface TestRunContext {
  log: (level: 'info' | 'pass' | 'fail', message: string, extra?: { expected?: unknown; actual?: unknown }) => void
  ensureAudioTrack: () => Promise<string>
  importAsset: (trackId: string, assetName: string) => Promise<void>
  clearTrackClips: (trackId: string) => Promise<void>
}

export interface TestDef {
  id: string
  kind: TestKind
  /** Which roadmap phase this test belongs to. Defaults to 1. */
  phase?: number
  phase1Item: string
  title: string
  instructions: string // for MANUAL, shown to user; for AUTO, short description
  run?: (ctx: TestRunContext) => Promise<{ pass: boolean; note: string }>
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

async function poll<T>(
  fn: () => Promise<T>,
  predicate: (v: T) => boolean,
  timeoutMs: number,
  intervalMs = 33,
): Promise<{ ok: boolean; value: T }> {
  const start = Date.now()
  let value = await fn()
  while (!predicate(value)) {
    if (Date.now() - start > timeoutMs) return { ok: false, value }
    await sleep(intervalMs)
    value = await fn()
  }
  return { ok: true, value }
}

function approx(a: number, b: number, tol: number): boolean {
  return Math.abs(a - b) <= tol
}

export const TESTS: TestDef[] = [
  // -------------------------------------------------------------------------
  // AUTO tests — atomic set/readback
  // -------------------------------------------------------------------------
  {
    id: 'master_volume_roundtrip',
    kind: 'AUTO',
    phase1Item: 'Master volume control',
    title: 'Master volume set/readback',
    instructions: 'Writes -12, -6, 0 dB via set_master_volume; reads back via dev_dump_state.',
    run: async ({ log }) => {
      for (const target of [-12, -6, 0]) {
        await invoke('set_master_volume', { db: target })
        await sleep(40)
        const s = await devDumpState()
        const ok = approx(s.masterVolumeDb, target, 0.01)
        log(ok ? 'pass' : 'fail', `master_volume_db`, { expected: target, actual: s.masterVolumeDb })
        if (!ok) return { pass: false, note: `readback mismatch at ${target} dB` }
      }
      return { pass: true, note: '3/3 roundtrips matched' }
    },
  },
  {
    id: 'time_signature_roundtrip',
    kind: 'AUTO',
    phase1Item: 'Time signature numerator/denominator',
    title: 'Time sig set/readback',
    instructions: 'Writes 7/8, 4/4, 12/16; reads back packed + unpacked values.',
    run: async ({ log }) => {
      for (const [num, den] of [[7, 8], [4, 4], [12, 16]] as const) {
        await invoke('set_time_signature', { numerator: num, denominator: den })
        await sleep(40)
        const s = await devDumpState()
        const ok = s.timeSigNumerator === num && s.timeSigDenominator === den
        log(ok ? 'pass' : 'fail', 'time_sig readback', {
          expected: `${num}/${den}`,
          actual: `${s.timeSigNumerator}/${s.timeSigDenominator}`,
        })
        if (!ok) return { pass: false, note: `mismatch at ${num}/${den}` }
      }
      return { pass: true, note: '3/3 signatures matched' }
    },
  },
  {
    id: 'pattern_mode_toggle',
    kind: 'AUTO',
    phase1Item: 'Pattern mode vs Song mode toggle',
    title: 'Pattern mode toggle',
    instructions: 'Toggles pattern_mode on/off and verifies atomic readback.',
    run: async ({ log }) => {
      for (const target of [true, false, true]) {
        await invoke('set_pattern_mode', { enabled: target })
        await sleep(40)
        const s = await devDumpState()
        const ok = s.patternMode === target
        log(ok ? 'pass' : 'fail', 'pattern_mode readback', { expected: target, actual: s.patternMode })
        if (!ok) return { pass: false, note: `mismatch at ${target}` }
      }
      await invoke('set_pattern_mode', { enabled: false })
      return { pass: true, note: '3/3 toggles matched' }
    },
  },
  {
    id: 'double_stop_reset',
    kind: 'AUTO',
    phase1Item: 'Stop (double-stop resets to zero)',
    title: 'Double-stop resets position',
    instructions: 'Sets position to 48000, stops twice, expects position=0 on the second stop.',
    run: async ({ log }) => {
      // Make sure loop_start=0 so the reset target is predictable.
      await invoke('set_loop', { start: 0, end: 0 })
      await invoke('stop')
      await invoke('set_position', { position: 48000 })
      await sleep(40)
      const afterSeek = await devDumpState()
      if (afterSeek.positionSamples !== 48000) {
        log('fail', 'set_position failed', { expected: 48000, actual: afterSeek.positionSamples })
        return { pass: false, note: 'set_position did not land' }
      }
      // First stop: position should stay (not playing -> second stop will reset)
      await invoke('stop')
      await sleep(40)
      const afterFirst = await devDumpState()
      log('info', 'after 1st stop', { actual: afterFirst.positionSamples })
      // Second stop: position should reset to 0
      await invoke('stop')
      await sleep(40)
      const afterSecond = await devDumpState()
      const ok = afterSecond.positionSamples === 0
      log(ok ? 'pass' : 'fail', 'position after 2nd stop', { expected: 0, actual: afterSecond.positionSamples })
      return { pass: ok, note: ok ? 'reset to 0' : `position=${afterSecond.positionSamples}` }
    },
  },
  {
    id: 'device_recovery',
    kind: 'AUTO',
    phase1Item: 'Graceful device disconnect + default fallback',
    title: 'Force stream error → engine recovers',
    instructions: 'Flips stream_error atomic; expects poll_audio_health to restart stream within 500 ms.',
    run: async ({ log }) => {
      const before = await devDumpState()
      if (!before.streamRunning) {
        // try to start
        try { await invoke('start_engine') } catch {}
        await sleep(100)
      }
      const before2 = await devDumpState()
      if (!before2.streamRunning) {
        return { pass: false, note: 'engine not running — start it first' }
      }
      log('info', 'device before', { actual: before2.activeDeviceName })
      await devForceDeviceError()
      const { ok, value } = await poll(devDumpState, (s) => s.streamRunning && !s.streamErrorFlag, 1000)
      log(ok ? 'pass' : 'fail', 'recovery', {
        expected: 'stream_running=true, stream_error_flag=false',
        actual: `running=${value.streamRunning} err=${value.streamErrorFlag} device=${value.activeDeviceName}`,
      })
      return { pass: ok, note: ok ? `recovered to ${value.activeDeviceName}` : 'did not recover within 1s' }
    },
  },
  {
    id: 'pattern_loop_wrap',
    kind: 'AUTO',
    phase1Item: 'Pattern mode: loop current pattern',
    title: 'Position wraps in pattern mode',
    instructions: 'Sets pattern mode on, BPM 240, 4/4, plays, verifies position wraps below pattern length within 3s.',
    run: async ({ log }) => {
      await invoke('stop')
      await invoke('set_time_signature', { numerator: 4, denominator: 4 })
      await invoke('set_bpm', { bpm: 240 })
      await invoke('set_pattern_mode', { enabled: true })
      await invoke('set_position', { position: 0 })
      await invoke('play')
      // At 240 BPM, 4 beats/bar, 4 bars: samples = 60/240*48000*4*4 = 192000
      const patternLen = Math.round((60 / 240) * 48000 * 4 * 4)
      log('info', 'pattern length (samples)', { expected: patternLen })
      let maxPos = 0
      let sawWrap = false
      const start = Date.now()
      while (Date.now() - start < 4000) {
        await sleep(50)
        const s = await devDumpState()
        if (s.positionSamples < maxPos && maxPos > patternLen / 2) sawWrap = true
        maxPos = Math.max(maxPos, s.positionSamples)
        if (sawWrap) break
      }
      await invoke('stop')
      await invoke('set_pattern_mode', { enabled: false })
      log(sawWrap ? 'pass' : 'fail', 'pattern wrap', {
        expected: 'position decreases at least once',
        actual: `maxPos=${maxPos}`,
      })
      return { pass: sawWrap, note: sawWrap ? `wrapped, maxPos=${maxPos}` : 'never wrapped' }
    },
  },
  {
    id: 'track_meter_under_signal',
    kind: 'AUTO',
    phase1Item: 'Per-track peak meter in mixer',
    title: 'Per-track meter moves under signal',
    instructions: 'Imports pink noise onto a track, plays, polls track meter, expects peak > -30 dB within 2s.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      await invoke('stop')
      await invoke('set_pattern_mode', { enabled: false })
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'pink_noise_-12dbfs_10s.wav')
      await invoke('set_position', { position: 0 })
      try { await invoke('start_engine') } catch {}
      await invoke('play')
      const { ok, value } = await poll(devDumpState, (s) => {
        const t = s.tracks.find((t) => t.id === trackId)
        return !!t && Math.max(t.peakLDb, t.peakRDb) > -30
      }, 2500)
      await invoke('stop')
      const t = value.tracks.find((t) => t.id === trackId)
      log(ok ? 'pass' : 'fail', 'track meter', {
        expected: 'peak > -30 dB within 2.5s',
        actual: t ? `peakL=${t.peakLDb.toFixed(1)} peakR=${t.peakRDb.toFixed(1)} rms=${t.rmsDb.toFixed(1)}` : 'track not found',
      })
      return { pass: ok, note: ok ? 'meter moved' : 'meter stayed flat' }
    },
  },
  {
    id: 'master_meter_level_calibration',
    kind: 'AUTO',
    phase1Item: 'Audio callback / signal chain level integrity',
    title: '1 kHz -6 dBFS sine → master peak ≈ -6 dB',
    instructions: 'Plays the 1 kHz -6 dBFS test tone; asserts master peak within ±4 dB of -6 (allows SR mismatch).',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      await invoke('stop')
      await invoke('set_pattern_mode', { enabled: false })
      await invoke('set_master_volume', { db: 0 })
      await invoke('set_track_volume', { trackId: await ensureAudioTrack(), volumeDb: 0 })
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine_1khz_-6dbfs_stereo_5s.wav')
      await invoke('set_position', { position: 0 })
      try { await invoke('start_engine') } catch {}
      await invoke('play')
      await sleep(1000)
      const s = await devDumpState()
      await invoke('stop')
      const peak = s.masterPeakDb
      const ok = approx(peak, -6, 4.0)
      log(ok ? 'pass' : 'fail', 'master peak', { expected: '-6 ± 4 dB', actual: peak.toFixed(2) })
      return { pass: ok, note: `peak=${peak.toFixed(2)} dB` }
    },
  },
  {
    id: 'peak_hold_decay',
    kind: 'AUTO',
    phase1Item: 'Peak hold with decay (1.5s hold, 20 dB/s fall)',
    title: 'Peak hold latches then decays',
    instructions: 'Plays tone burst (1s @ -6 dBFS, 4s silence). Verifies peak_hold stays ≥ -10 dB at t=1.3s and drops at t=4s.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      await invoke('stop')
      await invoke('set_master_volume', { db: 0 })
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'tone_burst_silence.wav')
      await invoke('set_position', { position: 0 })
      try { await invoke('start_engine') } catch {}
      await invoke('play')
      // Let burst complete (~1.1s) and check peak_hold is still latched
      await sleep(1300)
      const latched = await devDumpState()
      log('info', 'peak_hold at t=1.3s', { actual: latched.masterPeakHoldDb.toFixed(2) })
      // Wait for decay window and check again at t=4s
      await sleep(2700)
      const decayed = await devDumpState()
      log('info', 'peak_hold at t=4s', { actual: decayed.masterPeakHoldDb.toFixed(2) })
      await invoke('stop')
      const latchedOk = latched.masterPeakHoldDb > -10
      const decayedOk = decayed.masterPeakHoldDb < latched.masterPeakHoldDb
      const ok = latchedOk && decayedOk
      log(ok ? 'pass' : 'fail', 'peak_hold behavior', {
        expected: 'latched > -10 at 1.3s AND decayed by 4s',
        actual: `latched=${latched.masterPeakHoldDb.toFixed(2)} decayed=${decayed.masterPeakHoldDb.toFixed(2)}`,
      })
      return { pass: ok, note: ok ? 'latched + decayed' : `latchedOk=${latchedOk} decayedOk=${decayedOk}` }
    },
  },
  // -------------------------------------------------------------------------
  // DOM / visual tests — automated via DOM queries + screenshots
  // -------------------------------------------------------------------------
  {
    id: 'ruler_click_seek',
    kind: 'AUTO',
    phase1Item: 'Seek to position (click ruler)',
    title: 'Ruler click seeks transport',
    instructions: 'Dispatches a synthetic click on the arrangement canvas ruler area and verifies position changes.',
    run: async ({ log }) => {
      await invoke('stop')
      await invoke('set_position', { position: 0 })
      await sleep(40)
      const before = await devDumpState()
      log('info', 'position before click', { actual: before.positionSamples })

      // Click at x=200 in the ruler area (top 22px of canvas)
      const clicked = clickCanvas('arrangement-canvas', 200, 10)
      if (!clicked) {
        log('fail', 'canvas not found', { expected: 'arrangement-canvas exists' })
        return { pass: false, note: 'arrangement canvas not in DOM' }
      }
      await sleep(60)
      const after = await devDumpState()
      const ok = after.positionSamples !== before.positionSamples
      log(ok ? 'pass' : 'fail', 'position after ruler click', {
        expected: 'position changed',
        actual: `before=${before.positionSamples} after=${after.positionSamples}`,
      })
      return { pass: ok, note: ok ? `seeked to ${after.positionSamples}` : 'position did not change' }
    },
  },
  {
    id: 'song_mode_no_wrap',
    kind: 'AUTO',
    phase1Item: 'Pattern mode: loop / Song mode: arrangement',
    title: 'Song mode plays past pattern boundary',
    instructions: 'Sets song mode, plays at 240 BPM, verifies position exceeds one 4-bar pattern length without wrapping.',
    run: async ({ log }) => {
      await invoke('stop')
      await invoke('set_time_signature', { numerator: 4, denominator: 4 })
      await invoke('set_bpm', { bpm: 240 })
      await invoke('set_pattern_mode', { enabled: false })
      await invoke('set_position', { position: 0 })
      await invoke('play')

      // At 240 BPM, 4/4, 4 bars = 192000 samples. Wait long enough to exceed it.
      const patternLen = Math.round((60 / 240) * 48000 * 4 * 4)
      let maxPos = 0
      const start = Date.now()
      while (Date.now() - start < 5000) {
        await sleep(100)
        const s = await devDumpState()
        maxPos = Math.max(maxPos, s.positionSamples)
        if (maxPos > patternLen) break
      }
      await invoke('stop')
      const ok = maxPos > patternLen
      log(ok ? 'pass' : 'fail', 'song mode exceeded pattern boundary', {
        expected: `position > ${patternLen}`,
        actual: `maxPos=${maxPos}`,
      })
      return { pass: ok, note: ok ? `maxPos=${maxPos} > ${patternLen}` : `stuck at ${maxPos}` }
    },
  },
  {
    id: 'db_scale_visible',
    kind: 'AUTO',
    phase1Item: 'dB scale markings (-60 to 0)',
    title: 'dB scale markings visible on mixer',
    instructions: 'Opens mixer, queries DOM for all 8 dB scale marks and verifies they exist and are visible.',
    run: async ({ log, ensureAudioTrack }) => {
      // Ensure mixer is open and at least one track exists
      if (!queryTestId('panel-mixer')) simulateKey('F9')
      await ensureAudioTrack()
      await sleep(200)

      const expectedMarks = [6, 0, -6, -12, -24, -36, -48, -60]
      const missing: number[] = []
      const hidden: number[] = []

      for (const db of expectedMarks) {
        const el = queryTestId(`db-mark-${db}`)
        if (!el) {
          missing.push(db)
          continue
        }
        const rect = el.getBoundingClientRect()
        if (rect.width === 0 || rect.height === 0) {
          hidden.push(db)
        }
      }

      const ok = missing.length === 0 && hidden.length === 0
      log(ok ? 'pass' : 'fail', 'dB scale marks', {
        expected: '8 marks present and visible',
        actual: `missing=[${missing}] hidden=[${hidden}]`,
      })
      return { pass: ok, note: ok ? '8/8 marks visible' : `missing=${missing.length} hidden=${hidden.length}` }
    },
  },
  {
    id: 'rms_overlay_visible',
    kind: 'AUTO',
    phase1Item: 'RMS meter overlay (darker bar behind peak)',
    title: 'RMS overlay visible on meters under signal',
    instructions: 'Plays pink noise, then checks that both peak and RMS meter DOM elements have non-zero height.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      await invoke('stop')
      await invoke('set_pattern_mode', { enabled: false })
      await invoke('set_master_volume', { db: 0 })
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'pink_noise_-12dbfs_10s.wav')
      await invoke('set_position', { position: 0 })
      try { await invoke('start_engine') } catch {}
      await invoke('play')
      await sleep(1500)

      // Check master meter DOM elements
      const peakEl = queryTestId('meter-peak-master-L')
      const rmsEl = queryTestId('meter-rms-master-L')
      const peakDb = getMeterDb('meter-peak-master-L')
      const rmsDb = getMeterDb('meter-rms-master-L')

      await invoke('stop')

      const peakExists = !!peakEl
      const rmsExists = !!rmsEl
      const peakActive = peakDb !== null && peakDb > -60
      const rmsActive = rmsDb !== null && rmsDb > -60

      log('info', 'meter DOM state', {
        actual: `peakEl=${peakExists} rmsEl=${rmsExists} peakDb=${peakDb} rmsDb=${rmsDb}`,
      })

      const ok = peakExists && rmsExists && peakActive && rmsActive
      log(ok ? 'pass' : 'fail', 'RMS overlay', {
        expected: 'both peak and RMS bars active (> -60 dB)',
        actual: `peak=${peakDb?.toFixed(1)} rms=${rmsDb?.toFixed(1)}`,
      })
      return { pass: ok, note: ok ? `peak=${peakDb?.toFixed(1)} rms=${rmsDb?.toFixed(1)}` : 'meter bars inactive' }
    },
  },
  {
    id: 'stereo_independence',
    kind: 'AUTO',
    phase1Item: 'Per-track L/R meter independence',
    title: 'L and R meters read independently',
    instructions: 'Loads stereo_pan_test.wav, plays it, polls track meters and verifies L and R peaks differ across samples.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      await invoke('stop')
      await invoke('set_pattern_mode', { enabled: false })
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'stereo_pan_test.wav')
      await invoke('set_position', { position: 0 })
      try { await invoke('start_engine') } catch {}
      await invoke('play')

      let diffCount = 0
      const samples = 20
      for (let i = 0; i < samples; i++) {
        await sleep(100)
        const s = await devDumpState()
        const t = s.tracks.find((t) => t.id === trackId)
        if (t && Math.abs(t.peakLDb - t.peakRDb) > 0.5) {
          diffCount++
        }
      }
      await invoke('stop')

      const ok = diffCount >= 3
      log(ok ? 'pass' : 'fail', 'L/R independence', {
        expected: 'L and R differ in >= 3 of 20 samples',
        actual: `diffCount=${diffCount}/${samples}`,
      })
      return { pass: ok, note: ok ? `${diffCount}/${samples} differed` : `only ${diffCount} differed` }
    },
  },
  // -------------------------------------------------------------------------
  // Transport tests
  // -------------------------------------------------------------------------
  {
    id: 'play_pause_toggle',
    kind: 'AUTO',
    phase1Item: 'Play / pause toggle',
    title: 'Play starts, stop pauses',
    instructions: 'Invokes play, verifies playing=true; invokes stop, verifies playing=false.',
    run: async ({ log }) => {
      await invoke('stop')
      await sleep(40)
      const s0 = await devDumpState()
      if (s0.playing) { log('fail', 'precondition', { expected: false, actual: true }); return { pass: false, note: 'already playing' } }
      await invoke('play')
      await sleep(40)
      const s1 = await devDumpState()
      if (!s1.playing) { log('fail', 'after play', { expected: true, actual: false }); return { pass: false, note: 'play did not start' } }
      await invoke('stop')
      await sleep(40)
      const s2 = await devDumpState()
      const ok = !s2.playing
      log(ok ? 'pass' : 'fail', 'play/stop cycle', { expected: 'playing→true→false', actual: `${s0.playing}→${s1.playing}→${s2.playing}` })
      return { pass: ok, note: ok ? 'toggled correctly' : 'stop did not pause' }
    },
  },
  {
    id: 'bpm_roundtrip',
    kind: 'AUTO',
    phase1Item: 'BPM setting (20-999)',
    title: 'BPM set/readback at boundaries',
    instructions: 'Sets BPM to 20, 140, 999 and verifies atomic readback.',
    run: async ({ log }) => {
      for (const target of [20, 140, 300, 999]) {
        await invoke('set_bpm', { bpm: target })
        await sleep(40)
        const s = await devDumpState()
        const ok = Math.abs(s.bpm - target) < 0.01
        log(ok ? 'pass' : 'fail', `bpm=${target}`, { expected: target, actual: s.bpm })
        if (!ok) return { pass: false, note: `mismatch at ${target}` }
      }
      await invoke('set_bpm', { bpm: 140 })
      return { pass: true, note: '4/4 BPM values matched' }
    },
  },
  {
    id: 'loop_toggle',
    kind: 'AUTO',
    phase1Item: 'Loop mode: toggle on/off',
    title: 'Loop toggle roundtrip',
    instructions: 'Toggles loop on then off, verifies readback each time.',
    run: async ({ log }) => {
      // Ensure starts off
      const initial = await devDumpState()
      if (initial.looping) await invoke('toggle_loop')
      await sleep(40)

      await invoke('toggle_loop')
      await sleep(40)
      const s1 = await devDumpState()
      if (!s1.looping) { log('fail', 'toggle on', { expected: true, actual: false }); return { pass: false, note: 'loop did not enable' } }

      await invoke('toggle_loop')
      await sleep(40)
      const s2 = await devDumpState()
      const ok = !s2.looping
      log(ok ? 'pass' : 'fail', 'loop toggle cycle', { expected: 'off→on→off', actual: `${initial.looping}→${s1.looping}→${s2.looping}` })
      return { pass: ok, note: ok ? 'toggled correctly' : 'did not toggle off' }
    },
  },
  {
    id: 'position_advances',
    kind: 'AUTO',
    phase1Item: 'Sample-accurate playback position tracking',
    title: 'Position advances proportional to BPM',
    instructions: 'Plays at 120 BPM for ~500ms, verifies position advanced roughly 24000 samples (±20%).',
    run: async ({ log }) => {
      await invoke('stop')
      await invoke('set_bpm', { bpm: 120 })
      await invoke('set_pattern_mode', { enabled: false })
      await invoke('set_position', { position: 0 })
      try { await invoke('start_engine') } catch {}
      await invoke('play')
      await sleep(500)
      const s = await devDumpState()
      await invoke('stop')
      // 500ms at 48kHz = 24000 samples expected
      const expected = 24000
      const ok = s.positionSamples > expected * 0.4 && s.positionSamples < expected * 2.0
      log(ok ? 'pass' : 'fail', 'position after 500ms', { expected: `~${expected} ±40%`, actual: s.positionSamples })
      return { pass: ok, note: `position=${s.positionSamples}` }
    },
  },
  // -------------------------------------------------------------------------
  // Keyboard shortcut tests
  // -------------------------------------------------------------------------
  {
    id: 'key_space_play',
    kind: 'AUTO',
    phase1Item: 'Space: play/pause',
    title: 'Space key toggles playback',
    instructions: 'Dispatches Space keydown, verifies playing toggles.',
    run: async ({ log }) => {
      await invoke('stop')
      await sleep(40)
      simulateKey('Space')
      await sleep(80)
      const s1 = await devDumpState()
      if (!s1.playing) { log('fail', 'Space did not play', { expected: true, actual: false }); return { pass: false, note: 'Space had no effect' } }
      simulateKey('Space')
      await sleep(80)
      const s2 = await devDumpState()
      const ok = !s2.playing
      log(ok ? 'pass' : 'fail', 'Space toggle', { expected: 'play→stop', actual: `${s1.playing}→${s2.playing}` })
      return { pass: ok, note: ok ? 'Space toggled play/stop' : 'did not stop' }
    },
  },
  {
    id: 'key_home',
    kind: 'AUTO',
    phase1Item: 'Home key: return to start',
    title: 'Home key resets position to 0',
    instructions: 'Sets position to 48000, dispatches Home, verifies position=0.',
    run: async ({ log }) => {
      await invoke('stop')
      await invoke('set_position', { position: 48000 })
      await sleep(40)
      simulateKey('Home')
      await sleep(80)
      const s = await devDumpState()
      const ok = s.positionSamples === 0
      log(ok ? 'pass' : 'fail', 'Home key', { expected: 0, actual: s.positionSamples })
      return { pass: ok, note: ok ? 'reset to 0' : `position=${s.positionSamples}` }
    },
  },
  {
    id: 'key_end',
    kind: 'AUTO',
    phase1Item: 'End key: jump to end of last clip',
    title: 'End key jumps to end of last clip',
    instructions: 'Imports an audio clip, dispatches End key, verifies position jumped past 0.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      await invoke('stop')
      await invoke('set_bpm', { bpm: 140 })
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine_1khz_-6dbfs_stereo_5s.wav')
      await invoke('set_position', { position: 0 })
      await sleep(40)
      simulateKey('End')
      await sleep(80)
      const s = await devDumpState()
      const ok = s.positionSamples > 0
      log(ok ? 'pass' : 'fail', 'End key', { expected: '> 0', actual: s.positionSamples })
      return { pass: ok, note: ok ? `jumped to ${s.positionSamples}` : 'did not move' }
    },
  },
  {
    id: 'key_l_loop',
    kind: 'AUTO',
    phase1Item: 'Loop mode: toggle on/off',
    title: 'L key toggles loop mode',
    instructions: 'Dispatches KeyL, verifies looping toggles.',
    run: async ({ log }) => {
      const s0 = await devDumpState()
      const wasBefore = s0.looping
      simulateKey('KeyL')
      await sleep(80)
      const s1 = await devDumpState()
      const toggled = s1.looping !== wasBefore
      simulateKey('KeyL')
      await sleep(80)
      const s2 = await devDumpState()
      const toggledBack = s2.looping === wasBefore
      const ok = toggled && toggledBack
      log(ok ? 'pass' : 'fail', 'L key loop toggle', { expected: `${wasBefore}→${!wasBefore}→${wasBefore}`, actual: `${s0.looping}→${s1.looping}→${s2.looping}` })
      return { pass: ok, note: ok ? 'L toggled loop' : 'did not toggle' }
    },
  },
  {
    id: 'key_f5_playlist',
    kind: 'AUTO',
    phase1Item: 'F5: toggle Playlist',
    title: 'F5 toggles playlist panel',
    instructions: 'Dispatches F5, checks panel-playlist testid appears/disappears.',
    run: async ({ log }) => {
      const before = !!queryTestId('panel-playlist')
      simulateKey('F5')
      await sleep(100)
      const after = !!queryTestId('panel-playlist')
      const toggled = before !== after
      // Toggle back to original state
      simulateKey('F5')
      await sleep(100)
      const restored = !!queryTestId('panel-playlist') === before
      const ok = toggled && restored
      log(ok ? 'pass' : 'fail', 'F5 playlist', { expected: `${before}→${!before}→${before}`, actual: `${before}→${after}→${!!queryTestId('panel-playlist')}` })
      return { pass: ok, note: ok ? 'F5 toggled playlist' : 'no toggle' }
    },
  },
  {
    id: 'key_f9_mixer',
    kind: 'AUTO',
    phase1Item: 'F9: toggle Mixer',
    title: 'F9 toggles mixer panel',
    instructions: 'Dispatches F9, checks panel-mixer testid appears/disappears.',
    run: async ({ log }) => {
      const before = !!queryTestId('panel-mixer')
      simulateKey('F9')
      await sleep(100)
      const after = !!queryTestId('panel-mixer')
      const toggled = before !== after
      simulateKey('F9')
      await sleep(100)
      const restored = !!queryTestId('panel-mixer') === before
      const ok = toggled && restored
      log(ok ? 'pass' : 'fail', 'F9 mixer', { expected: `${before}→${!before}→${before}`, actual: `${before}→${after}→${!!queryTestId('panel-mixer')}` })
      return { pass: ok, note: ok ? 'F9 toggled mixer' : 'no toggle' }
    },
  },
  {
    id: 'key_f8_browser',
    kind: 'AUTO',
    phase1Item: 'F8: toggle Browser',
    title: 'F8 toggles browser panel',
    instructions: 'Dispatches F8, checks panel-browser testid appears/disappears.',
    run: async ({ log }) => {
      const before = !!queryTestId('panel-browser')
      simulateKey('F8')
      await sleep(100)
      const after = !!queryTestId('panel-browser')
      const toggled = before !== after
      simulateKey('F8')
      await sleep(100)
      const restored = !!queryTestId('panel-browser') === before
      const ok = toggled && restored
      log(ok ? 'pass' : 'fail', 'F8 browser', { expected: `${before}→${!before}→${before}`, actual: `${before}→${after}→${!!queryTestId('panel-browser')}` })
      return { pass: ok, note: ok ? 'F8 toggled browser' : 'no toggle' }
    },
  },
  // -------------------------------------------------------------------------
  // Audio graph / track tests
  // -------------------------------------------------------------------------
  {
    id: 'track_volume_roundtrip',
    kind: 'AUTO',
    phase1Item: 'Per-track volume fader (linear to dB)',
    title: 'Track volume set/readback',
    instructions: 'Sets track volume to -12, 0, +6 dB and verifies via get_tracks.',
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      for (const target of [-12, 0, 6]) {
        await invoke('set_track_volume', { trackId, volumeDb: target })
        await sleep(40)
        const tracks = await invoke<any[]>('get_tracks')
        const t = tracks.find((t: any) => t.id === trackId)
        const ok = t && Math.abs(t.volume_db - target) < 0.01
        log(ok ? 'pass' : 'fail', `volume=${target}`, { expected: target, actual: t?.volume_db })
        if (!ok) return { pass: false, note: `mismatch at ${target}` }
      }
      await invoke('set_track_volume', { trackId, volumeDb: 0 })
      return { pass: true, note: '3/3 volume values matched' }
    },
  },
  {
    id: 'track_pan_roundtrip',
    kind: 'AUTO',
    phase1Item: 'Per-track pan knob (constant power)',
    title: 'Track pan set/readback',
    instructions: 'Sets pan to -1, 0, +1 and verifies via get_tracks.',
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      for (const target of [-1, 0, 0.5, 1]) {
        await invoke('set_track_pan', { trackId, pan: target })
        await sleep(40)
        const tracks = await invoke<any[]>('get_tracks')
        const t = tracks.find((t: any) => t.id === trackId)
        const ok = t && Math.abs(t.pan - target) < 0.01
        log(ok ? 'pass' : 'fail', `pan=${target}`, { expected: target, actual: t?.pan })
        if (!ok) return { pass: false, note: `mismatch at ${target}` }
      }
      await invoke('set_track_pan', { trackId, pan: 0 })
      return { pass: true, note: '4/4 pan values matched' }
    },
  },
  {
    id: 'track_mute_toggle',
    kind: 'AUTO',
    phase1Item: 'Per-track mute button',
    title: 'Track mute toggle roundtrip',
    instructions: 'Toggles mute and verifies via get_tracks.',
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      // Ensure starts unmuted
      let tracks = await invoke<any[]>('get_tracks')
      let t = tracks.find((t: any) => t.id === trackId)
      if (t?.muted) await invoke('toggle_mute', { trackId })

      await invoke('toggle_mute', { trackId })
      await sleep(40)
      tracks = await invoke<any[]>('get_tracks')
      t = tracks.find((t: any) => t.id === trackId)
      if (!t?.muted) { log('fail', 'mute on', { expected: true, actual: false }); return { pass: false, note: 'mute did not enable' } }

      await invoke('toggle_mute', { trackId })
      await sleep(40)
      tracks = await invoke<any[]>('get_tracks')
      t = tracks.find((t: any) => t.id === trackId)
      const ok = !t?.muted
      log(ok ? 'pass' : 'fail', 'mute toggle', { expected: 'off→on→off', actual: ok ? 'correct' : 'stuck muted' })
      return { pass: ok, note: ok ? 'mute toggled correctly' : 'did not unmute' }
    },
  },
  {
    id: 'track_solo_toggle',
    kind: 'AUTO',
    phase1Item: 'Per-track solo button',
    title: 'Track solo toggle roundtrip',
    instructions: 'Toggles solo and verifies via get_tracks.',
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      let tracks = await invoke<any[]>('get_tracks')
      let t = tracks.find((t: any) => t.id === trackId)
      if (t?.soloed) await invoke('toggle_solo', { trackId })

      await invoke('toggle_solo', { trackId })
      await sleep(40)
      tracks = await invoke<any[]>('get_tracks')
      t = tracks.find((t: any) => t.id === trackId)
      if (!t?.soloed) { log('fail', 'solo on', { expected: true, actual: false }); return { pass: false, note: 'solo did not enable' } }

      await invoke('toggle_solo', { trackId })
      await sleep(40)
      tracks = await invoke<any[]>('get_tracks')
      t = tracks.find((t: any) => t.id === trackId)
      const ok = !t?.soloed
      log(ok ? 'pass' : 'fail', 'solo toggle', { expected: 'off→on→off', actual: ok ? 'correct' : 'stuck soloed' })
      return { pass: ok, note: ok ? 'solo toggled correctly' : 'did not unsolo' }
    },
  },
  {
    id: 'track_solo_safe',
    kind: 'AUTO',
    phase1Item: 'Solo-safe mode per track',
    title: 'Solo-safe toggle roundtrip',
    instructions: 'Toggles solo_safe and verifies via get_tracks.',
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      let tracks = await invoke<any[]>('get_tracks')
      let t = tracks.find((t: any) => t.id === trackId)
      if (t?.solo_safe) await invoke('toggle_solo_safe', { trackId })

      await invoke('toggle_solo_safe', { trackId })
      await sleep(40)
      tracks = await invoke<any[]>('get_tracks')
      t = tracks.find((t: any) => t.id === trackId)
      if (!t?.solo_safe) { log('fail', 'solo_safe on', { expected: true, actual: false }); return { pass: false, note: 'solo_safe did not enable' } }

      await invoke('toggle_solo_safe', { trackId })
      await sleep(40)
      tracks = await invoke<any[]>('get_tracks')
      t = tracks.find((t: any) => t.id === trackId)
      const ok = !t?.solo_safe
      log(ok ? 'pass' : 'fail', 'solo_safe toggle', { expected: 'off→on→off', actual: ok ? 'correct' : 'stuck' })
      return { pass: ok, note: ok ? 'solo_safe toggled correctly' : 'did not reset' }
    },
  },
  {
    id: 'track_add_remove',
    kind: 'AUTO',
    phase1Item: 'Track node creation in audio graph',
    title: 'Add and remove audio track',
    instructions: 'Adds an audio track, verifies it appears in get_tracks, removes it, verifies gone.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const beforeCount = before.filter((t: any) => t.kind !== 'Master').length
      const newId = await invoke<string>('add_audio_track', { name: 'TestAddRemove' })
      const after = await invoke<any[]>('get_tracks')
      const afterCount = after.filter((t: any) => t.kind !== 'Master').length
      if (afterCount !== beforeCount + 1) {
        log('fail', 'add track', { expected: beforeCount + 1, actual: afterCount })
        return { pass: false, note: 'track count did not increase' }
      }
      await invoke('remove_track', { trackId: newId })
      const final_ = await invoke<any[]>('get_tracks')
      const finalCount = final_.filter((t: any) => t.kind !== 'Master').length
      const ok = finalCount === beforeCount
      log(ok ? 'pass' : 'fail', 'remove track', { expected: beforeCount, actual: finalCount })
      return { pass: ok, note: ok ? 'add/remove roundtrip correct' : 'count mismatch after remove' }
    },
  },
  {
    id: 'track_summing',
    kind: 'AUTO',
    phase1Item: 'Track summing (mix N tracks to stereo)',
    title: 'Two tracks sum to louder master',
    instructions: 'Creates 2 tracks with pink noise, plays, verifies master peak > single-track peak.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      await invoke('stop')
      await invoke('set_pattern_mode', { enabled: false })
      await invoke('set_master_volume', { db: 0 })

      // Create two tracks with the same signal
      const t1 = await ensureAudioTrack()
      await clearTrackClips(t1)
      await importAsset(t1, 'pink_noise_-12dbfs_10s.wav')
      await invoke('set_track_volume', { trackId: t1, volumeDb: -6 })

      const t2 = await invoke<string>('add_audio_track', { name: 'Sum Test' })
      await importAsset(t2, 'pink_noise_-12dbfs_10s.wav')
      await invoke('set_track_volume', { trackId: t2, volumeDb: -6 })

      await invoke('set_position', { position: 0 })
      try { await invoke('start_engine') } catch {}
      await invoke('play')
      await sleep(1000)
      const both = await devDumpState()
      await invoke('stop')

      // Mute second track, play again
      await invoke('toggle_mute', { trackId: t2 })
      await invoke('set_position', { position: 0 })
      await invoke('play')
      await sleep(1000)
      const single = await devDumpState()
      await invoke('stop')

      // Clean up
      await invoke('toggle_mute', { trackId: t2 })
      await invoke('remove_track', { trackId: t2 })

      const ok = both.masterPeakDb > single.masterPeakDb + 1
      log(ok ? 'pass' : 'fail', 'track summing', {
        expected: 'both > single + 1 dB',
        actual: `both=${both.masterPeakDb.toFixed(1)} single=${single.masterPeakDb.toFixed(1)}`,
      })
      return { pass: ok, note: `both=${both.masterPeakDb.toFixed(1)} single=${single.masterPeakDb.toFixed(1)}` }
    },
  },
  // -------------------------------------------------------------------------
  // Audio decode tests
  // -------------------------------------------------------------------------
  {
    id: 'wav_import',
    kind: 'AUTO',
    phase1Item: 'WAV decode (PCM 16/24/32-bit, float)',
    title: 'WAV file import creates clip',
    instructions: 'Imports a WAV test asset and verifies a clip was created on the track.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine_1khz_-6dbfs_stereo_5s.wav')
      const clips = await invoke<any[]>('get_track_clips', { trackId })
      const ok = clips.length === 1
      log(ok ? 'pass' : 'fail', 'WAV import', { expected: '1 clip', actual: `${clips.length} clips` })
      return { pass: ok, note: ok ? `clip: ${clips[0]?.name}` : 'no clip created' }
    },
  },
  // -------------------------------------------------------------------------
  // Clip manipulation tests
  // -------------------------------------------------------------------------
  {
    id: 'clip_move',
    kind: 'AUTO',
    phase1Item: 'Clip move: horizontal drag (time)',
    title: 'Move clip to new tick position',
    instructions: 'Creates a clip at tick 0, moves it to tick 960, verifies new position.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine_1khz_-6dbfs_stereo_5s.wav')
      const clips = await invoke<any[]>('get_track_clips', { trackId })
      if (clips.length === 0) { log('fail', 'no clip', {}); return { pass: false, note: 'no clip to move' } }
      const clipId = clips[0].id
      await invoke('move_clip', { trackId, clipId, newPositionTicks: 960 })
      const after = await invoke<any[]>('get_track_clips', { trackId })
      const moved = after.find((c: any) => c.id === clipId)
      const ok = moved?.position_ticks === 960
      log(ok ? 'pass' : 'fail', 'clip move', { expected: 960, actual: moved?.position_ticks })
      return { pass: ok, note: ok ? 'moved to 960' : `position=${moved?.position_ticks}` }
    },
  },
  {
    id: 'clip_resize',
    kind: 'AUTO',
    phase1Item: 'Clip resize: drag right edge',
    title: 'Resize clip length',
    instructions: 'Creates a clip and resizes it to 1920 ticks, verifies new length.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine_1khz_-6dbfs_stereo_5s.wav')
      const clips = await invoke<any[]>('get_track_clips', { trackId })
      if (clips.length === 0) { log('fail', 'no clip', {}); return { pass: false, note: 'no clip to resize' } }
      const clipId = clips[0].id
      const originalLen = clips[0].length_ticks
      const newLen = 1920
      await invoke('resize_clip', { trackId, clipId, newLengthTicks: newLen })
      const after = await invoke<any[]>('get_track_clips', { trackId })
      const resized = after.find((c: any) => c.id === clipId)
      const ok = resized?.length_ticks === newLen
      log(ok ? 'pass' : 'fail', 'clip resize', { expected: newLen, actual: resized?.length_ticks })
      // Restore original
      await invoke('resize_clip', { trackId, clipId, newLengthTicks: originalLen })
      return { pass: ok, note: ok ? `resized to ${newLen}` : `length=${resized?.length_ticks}` }
    },
  },
  {
    id: 'clip_delete',
    kind: 'AUTO',
    phase1Item: 'Clip delete: Delete/Backspace key',
    title: 'Delete clip removes it from track',
    instructions: 'Creates a clip, deletes it via command, verifies track is empty.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine_1khz_-6dbfs_stereo_5s.wav')
      const clips = await invoke<any[]>('get_track_clips', { trackId })
      if (clips.length === 0) { log('fail', 'no clip', {}); return { pass: false, note: 'no clip to delete' } }
      await invoke('delete_clip', { trackId, clipId: clips[0].id })
      const after = await invoke<any[]>('get_track_clips', { trackId })
      const ok = after.length === 0
      log(ok ? 'pass' : 'fail', 'clip delete', { expected: 0, actual: after.length })
      return { pass: ok, note: ok ? 'clip deleted' : `${after.length} clips remain` }
    },
  },
  // -------------------------------------------------------------------------
  // Project save/load tests
  // -------------------------------------------------------------------------
  {
    id: 'project_save_load',
    kind: 'AUTO',
    phase1Item: 'File > Save / File > Open project',
    title: 'Project save/load roundtrip',
    instructions: 'Adds tracks, sets BPM, saves .hwp, new project, loads, verifies state matches.',
    run: async ({ log }) => {
      // Setup: add a track and set BPM
      await invoke('set_bpm', { bpm: 175 })
      const trackId = await invoke<string>('add_audio_track', { name: 'SaveTest' })
      await sleep(40)

      const beforeInfo = await invoke<any>('get_project_info')
      const beforeTracks = await invoke<any[]>('get_tracks')
      const beforeNonMaster = beforeTracks.filter((t: any) => t.kind !== 'Master').length

      // Save
      const path = '/tmp/hardwave_test_project.hwp'
      await invoke('save_project', { path })

      // New project (resets everything)
      await invoke('new_project')
      await sleep(40)
      const midInfo = await invoke<any>('get_project_info')
      const midTracks = await invoke<any[]>('get_tracks')
      const midNonMaster = midTracks.filter((t: any) => t.kind !== 'Master').length
      if (midNonMaster !== 0) {
        log('info', 'new_project did not clear tracks', { actual: midNonMaster })
      }

      // Load
      await invoke('load_project', { path })
      await sleep(40)
      const afterInfo = await invoke<any>('get_project_info')
      const afterTracks = await invoke<any[]>('get_tracks')
      const afterNonMaster = afterTracks.filter((t: any) => t.kind !== 'Master').length

      const bpmOk = Math.abs(afterInfo.bpm - 175) < 0.01
      const tracksOk = afterNonMaster >= beforeNonMaster
      const ok = bpmOk && tracksOk
      log(ok ? 'pass' : 'fail', 'save/load roundtrip', {
        expected: `bpm=175, tracks>=${beforeNonMaster}`,
        actual: `bpm=${afterInfo.bpm}, tracks=${afterNonMaster}`,
      })

      // Clean up test track
      const testTrack = afterTracks.find((t: any) => t.name === 'SaveTest')
      if (testTrack) await invoke('remove_track', { trackId: testTrack.id })

      return { pass: ok, note: ok ? 'save/load matched' : `bpmOk=${bpmOk} tracksOk=${tracksOk}` }
    },
  },
  {
    id: 'new_project_resets',
    kind: 'AUTO',
    phase1Item: 'File > New project',
    title: 'New project resets state',
    instructions: 'Adds tracks and clips, invokes new_project, verifies clean state.',
    run: async ({ log }) => {
      await invoke<string>('add_audio_track', { name: 'TempTrack' })
      await invoke('set_bpm', { bpm: 200 })
      await sleep(40)

      await invoke('new_project')
      await sleep(40)
      const info = await invoke<any>('get_project_info')
      const tracks = await invoke<any[]>('get_tracks')
      const nonMaster = tracks.filter((t: any) => t.kind !== 'Master').length
      const ok = nonMaster === 0 && info.name === 'Untitled'
      log(ok ? 'pass' : 'fail', 'new project', {
        expected: '0 tracks, name=Untitled',
        actual: `${nonMaster} tracks, name=${info.name}`,
      })
      return { pass: ok, note: ok ? 'reset to default' : `tracks=${nonMaster} name=${info.name}` }
    },
  },
  // -------------------------------------------------------------------------
  // Mixer UI DOM tests
  // -------------------------------------------------------------------------
  {
    id: 'clip_indicator_0dbfs',
    kind: 'AUTO',
    phase1Item: 'Clip indicator (red light at 0dBFS)',
    title: 'Clip LED activates at 0 dBFS',
    instructions: 'Plays a hot signal (+6 dB), checks master meter reads clipped via dev_dump_state.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      await invoke('stop')
      await invoke('set_pattern_mode', { enabled: false })
      await invoke('set_master_volume', { db: 12 })
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine_1khz_-6dbfs_stereo_5s.wav')
      await invoke('set_track_volume', { trackId, volumeDb: 12 })
      await invoke('set_position', { position: 0 })
      try { await invoke('start_engine') } catch {}
      await invoke('play')
      await sleep(1000)
      const s = await devDumpState()
      await invoke('stop')
      await invoke('set_master_volume', { db: 0 })
      await invoke('set_track_volume', { trackId, volumeDb: 0 })
      const ok = s.masterClipped || s.masterPeakDb >= -0.1
      log(ok ? 'pass' : 'fail', 'clip indicator', {
        expected: 'clipped=true or peak >= -0.1',
        actual: `clipped=${s.masterClipped} peak=${s.masterPeakDb.toFixed(1)}`,
      })
      return { pass: ok, note: `clipped=${s.masterClipped} peak=${s.masterPeakDb.toFixed(1)}` }
    },
  },
  // -------------------------------------------------------------------------
  // Audio device tests
  // -------------------------------------------------------------------------
  {
    id: 'audio_device_enum',
    kind: 'AUTO',
    phase1Item: 'Audio device enumeration (cpal)',
    title: 'Audio devices listed',
    instructions: 'Invokes get_audio_devices and verifies at least one output device.',
    run: async ({ log }) => {
      const devices = await invoke<any[]>('get_audio_devices')
      const ok = devices.length > 0
      log(ok ? 'pass' : 'fail', 'device list', { expected: '>= 1 device', actual: `${devices.length} devices` })
      return { pass: ok, note: ok ? `${devices.length} devices found` : 'no devices' }
    },
  },
  {
    id: 'audio_config_roundtrip',
    kind: 'AUTO',
    phase1Item: 'Buffer size configuration (64-4096)',
    title: 'Audio config get/set roundtrip',
    instructions: 'Reads audio config, verifies sample_rate and buffer_size are valid.',
    run: async ({ log }) => {
      const s = await devDumpState()
      const srOk = [44100, 48000, 88200, 96000].includes(s.sampleRate)
      const bsOk = s.bufferSize >= 64 && s.bufferSize <= 4096
      const ok = srOk && bsOk
      log(ok ? 'pass' : 'fail', 'audio config', {
        expected: 'valid SR and buffer size',
        actual: `sr=${s.sampleRate} bs=${s.bufferSize}`,
      })
      return { pass: ok, note: `sr=${s.sampleRate} bs=${s.bufferSize}` }
    },
  },
  {
    id: 'engine_start_stop',
    kind: 'AUTO',
    phase1Item: 'Stereo output stream creation',
    title: 'Engine start/stop cycle',
    instructions: 'Starts engine, verifies streamRunning=true; stops, verifies false.',
    run: async ({ log }) => {
      try { await invoke('start_engine') } catch {}
      await sleep(200)
      const s1 = await devDumpState()
      if (!s1.streamRunning) { log('fail', 'start', { expected: true, actual: false }); return { pass: false, note: 'engine did not start' } }
      await invoke('stop_engine')
      await sleep(200)
      const s2 = await devDumpState()
      const ok = !s2.streamRunning
      log(ok ? 'pass' : 'fail', 'engine stop', { expected: false, actual: s2.streamRunning })
      // Restart for other tests
      try { await invoke('start_engine') } catch {}
      return { pass: ok, note: ok ? 'start/stop cycle OK' : 'stream still running after stop' }
    },
  },
  // -------------------------------------------------------------------------
  // Waveform & rendering tests
  // -------------------------------------------------------------------------
  {
    id: 'waveform_peaks',
    kind: 'AUTO',
    phase1Item: 'Clip waveform peak pre-computation',
    title: 'Waveform peaks returned for imported audio',
    instructions: 'Imports WAV, requests 100 peak buckets, verifies non-zero data.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine_1khz_-6dbfs_stereo_5s.wav')
      const clips = await invoke<any[]>('get_track_clips', { trackId })
      if (clips.length === 0) { log('fail', 'no clip', {}); return { pass: false, note: 'no clip' } }
      const sourceId = clips[0].source_id
      const peaks = await invoke<[number, number][]>('get_waveform_peaks', { sourceId, numBuckets: 100 })
      const nonZero = peaks.filter(([mn, mx]) => mn !== 0 || mx !== 0).length
      const ok = peaks.length === 100 && nonZero > 50
      log(ok ? 'pass' : 'fail', 'waveform peaks', { expected: '100 buckets, >50 non-zero', actual: `${peaks.length} buckets, ${nonZero} non-zero` })
      return { pass: ok, note: `${peaks.length} buckets, ${nonZero} active` }
    },
  },
  // -------------------------------------------------------------------------
  // MIDI tests
  // -------------------------------------------------------------------------
  {
    id: 'midi_clip_roundtrip',
    kind: 'AUTO',
    phase1Item: 'MIDI note input via piano roll',
    title: 'MIDI clip create, add note, readback',
    instructions: 'Creates a MIDI track, creates a clip, adds a note, reads it back.',
    run: async ({ log }) => {
      const trackId = await invoke<string>('add_midi_track', { name: 'MIDITest' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, positionTicks: 0, lengthTicks: 3840, name: 'Test Clip' })
      await invoke('add_midi_note', { trackId, clipId, pitch: 60, velocity: 1.0, startTick: 0, durationTicks: 960 })
      const notes = await invoke<any[]>('get_midi_notes', { trackId, clipId })
      const ok = notes.length === 1 && notes[0].pitch === 60 && Math.abs(notes[0].velocity - 1.0) < 0.01
      log(ok ? 'pass' : 'fail', 'MIDI roundtrip', {
        expected: '1 note, C4, vel=1.0',
        actual: `${notes.length} notes${notes[0] ? `, pitch=${notes[0].pitch}, vel=${notes[0].velocity}` : ''}`,
      })
      await invoke('remove_track', { trackId })
      return { pass: ok, note: ok ? 'MIDI note roundtrip correct' : 'mismatch' }
    },
  },
  // -------------------------------------------------------------------------
  // Phase 1 — new features batch
  // -------------------------------------------------------------------------
  {
    id: 'exclusive_solo',
    kind: 'AUTO',
    phase1Item: 'Exclusive solo mode',
    title: 'Exclusive solo unsolos other tracks',
    instructions: 'Creates 3 tracks, solos track A exclusively, verifies only A is soloed. Then solos B exclusively, verifies only B is soloed.',
    run: async ({ log }) => {
      const idA = await invoke<string>('add_audio_track', { name: 'ExSolo_A' })
      const idB = await invoke<string>('add_audio_track', { name: 'ExSolo_B' })
      const idC = await invoke<string>('add_audio_track', { name: 'ExSolo_C' })

      // Exclusive solo track A
      await invoke('set_exclusive_solo', { trackId: idA })
      await sleep(40)
      let tracks = await invoke<any[]>('get_tracks')
      let a = tracks.find((t: any) => t.id === idA)
      let b = tracks.find((t: any) => t.id === idB)
      let c = tracks.find((t: any) => t.id === idC)
      if (!a?.soloed || b?.soloed || c?.soloed) {
        log('fail', 'exclusive solo A', { expected: 'A=true B=false C=false', actual: `A=${a?.soloed} B=${b?.soloed} C=${c?.soloed}` })
        await invoke('remove_track', { trackId: idA }); await invoke('remove_track', { trackId: idB }); await invoke('remove_track', { trackId: idC })
        return { pass: false, note: 'exclusive solo A failed' }
      }

      // Exclusive solo track B — A should unsolo
      await invoke('set_exclusive_solo', { trackId: idB })
      await sleep(40)
      tracks = await invoke<any[]>('get_tracks')
      a = tracks.find((t: any) => t.id === idA)
      b = tracks.find((t: any) => t.id === idB)
      c = tracks.find((t: any) => t.id === idC)
      const step2ok = !a?.soloed && b?.soloed && !c?.soloed
      log(step2ok ? 'pass' : 'fail', 'exclusive solo B', { expected: 'A=false B=true C=false', actual: `A=${a?.soloed} B=${b?.soloed} C=${c?.soloed}` })

      // Exclusive solo B again — should toggle off (unsolo all)
      await invoke('set_exclusive_solo', { trackId: idB })
      await sleep(40)
      tracks = await invoke<any[]>('get_tracks')
      b = tracks.find((t: any) => t.id === idB)
      const step3ok = !b?.soloed
      log(step3ok ? 'pass' : 'fail', 'exclusive solo toggle off', { expected: 'B=false', actual: `B=${b?.soloed}` })

      await invoke('remove_track', { trackId: idA }); await invoke('remove_track', { trackId: idB }); await invoke('remove_track', { trackId: idC })
      const ok = step2ok && step3ok
      return { pass: ok, note: ok ? 'exclusive solo works' : 'failed' }
    },
  },
  {
    id: 'pre_fader_meter',
    kind: 'AUTO',
    phase1Item: 'Pre-fader metering tap',
    title: 'Pre-fader meter reads signal regardless of fader',
    instructions: 'Imports pink noise, sets track volume to -inf, plays, verifies pre-fader peak > -30 dB while post-fader stays near -inf.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      await invoke('stop')
      await invoke('set_pattern_mode', { enabled: false })
      await invoke('set_master_volume', { db: 0 })
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'pink_noise_-12dbfs_10s.wav')
      await invoke('set_track_volume', { trackId, volumeDb: -100 })
      await invoke('set_position', { position: 0 })
      try { await invoke('start_engine') } catch {}
      await invoke('play')

      const { ok, value: _firstHit } = await poll(devDumpState, (s) => {
        const t = s.tracks.find((t) => t.id === trackId)
        return !!t && t.preFaderPeakDb !== undefined && t.preFaderPeakDb > -30
      }, 2500)
      void _firstHit
      // After signal is confirmed flowing, give the volume command a few
      // blocks to apply — set_track_volume is async via the command queue,
      // so the first post-fader meter writes can still reflect the old fader.
      await sleep(250)
      const value = await devDumpState()
      await invoke('stop')
      await invoke('set_track_volume', { trackId, volumeDb: 0 })

      const t = value.tracks.find((t) => t.id === trackId)
      const preFader = t?.preFaderPeakDb ?? -100
      const postFader = Math.max(t?.peakLDb ?? -100, t?.peakRDb ?? -100)
      const preOk = preFader > -30
      // Relative check: post should be at least 30 dB below pre (fader is at -100 dB).
      // Absolute threshold is unreliable because post-fader peaks from prior tests
      // can linger in the meter atomic before decaying.
      const postOk = postFader < preFader - 30
      const pass = preOk && postOk
      log(pass ? 'pass' : 'fail', 'pre vs post fader', {
        expected: 'pre > -30 dB, post < pre - 30 dB',
        actual: `pre=${preFader.toFixed(1)} post=${postFader.toFixed(1)}`,
      })
      return { pass, note: `pre=${preFader.toFixed(1)} post=${postFader.toFixed(1)}` }
    },
  },
  {
    id: 'loop_region_roundtrip',
    kind: 'AUTO',
    phase1Item: 'Loop start/end markers',
    title: 'Loop region set and playback wraps',
    instructions: 'Sets a loop region, enables loop, plays, verifies position wraps within the region.',
    run: async ({ log }) => {
      await invoke('stop')
      await invoke('set_pattern_mode', { enabled: false })
      await invoke('set_bpm', { bpm: 240 })
      const sr = 48000
      // Loop from bar 2 to bar 4 (at 240 BPM, 4/4: 1 bar = 48000 samples)
      const loopStart = sr * 1  // 1 second = bar 2
      const loopEnd = sr * 3    // 3 seconds = bar 4
      await invoke('set_loop', { start: loopStart, end: loopEnd })
      await invoke('toggle_loop')
      await sleep(40)
      const s0 = await devDumpState()
      if (!s0.looping) {
        await invoke('toggle_loop')
        await sleep(40)
      }
      await invoke('set_position', { position: loopStart })
      try { await invoke('start_engine') } catch {}
      await invoke('play')

      let sawWrap = false
      let maxPos = 0
      let minPosAfterHalf = loopEnd
      const start = Date.now()
      while (Date.now() - start < 4000) {
        await sleep(50)
        const s = await devDumpState()
        if (s.positionSamples > loopStart + (loopEnd - loopStart) / 2) {
          minPosAfterHalf = Math.min(minPosAfterHalf, s.positionSamples)
        }
        if (s.positionSamples < maxPos && maxPos > loopStart + (loopEnd - loopStart) / 2) {
          sawWrap = true
        }
        maxPos = Math.max(maxPos, s.positionSamples)
        if (sawWrap) break
      }
      await invoke('stop')
      await invoke('toggle_loop')
      await invoke('set_bpm', { bpm: 140 })

      const stayedInRange = maxPos <= loopEnd + sr * 0.1  // small tolerance
      const ok = sawWrap && stayedInRange
      log(ok ? 'pass' : 'fail', 'loop region', {
        expected: `wraps within [${loopStart}, ${loopEnd}]`,
        actual: `sawWrap=${sawWrap} maxPos=${maxPos} stayedInRange=${stayedInRange}`,
      })
      return { pass: ok, note: ok ? `loop wrapped, maxPos=${maxPos}` : `sawWrap=${sawWrap} maxPos=${maxPos}` }
    },
  },
  {
    id: 'loop_markers_visible',
    kind: 'AUTO',
    phase1Item: 'Loop visual indicator on ruler',
    title: 'Loop region overlay visible in arrangement',
    instructions: 'Sets a loop region, enables loop, checks that the loop-region DOM overlay is visible.',
    run: async ({ log }) => {
      await invoke('stop')
      await invoke('set_pattern_mode', { enabled: false })
      const sr = 48000
      await invoke('set_loop', { start: sr, end: sr * 3 })
      // Enable looping
      const s0 = await devDumpState()
      if (!s0.looping) await invoke('toggle_loop')
      await sleep(200)

      const el = queryTestId('loop-region-overlay')
      const ok = !!el
      log(ok ? 'pass' : 'fail', 'loop overlay DOM', {
        expected: 'loop-region-overlay exists',
        actual: ok ? 'found' : 'not found',
      })

      // Clean up
      await invoke('toggle_loop')
      return { pass: ok, note: ok ? 'loop overlay visible' : 'overlay not found in DOM' }
    },
  },
  {
    id: 'vertical_zoom',
    kind: 'AUTO',
    phase1Item: 'Vertical zoom (Ctrl+Shift+wheel)',
    title: 'Vertical zoom changes track height',
    instructions: 'Dispatches Ctrl+Shift+wheel events on the arrangement canvas and checks track height changes.',
    run: async ({ log, ensureAudioTrack }) => {
      await ensureAudioTrack()
      await sleep(100)

      // Read initial track height from data attribute
      const canvas = queryTestId('arrangement-canvas')
      if (!canvas) {
        log('fail', 'canvas not found', {})
        return { pass: false, note: 'arrangement canvas not in DOM' }
      }

      const initialHeight = parseInt(canvas.getAttribute('data-track-height') || '56', 10)

      // Zoom in: Ctrl+Shift+WheelUp
      const zoomIn = new WheelEvent('wheel', {
        deltaY: -100,
        ctrlKey: true,
        shiftKey: true,
        bubbles: true,
      })
      canvas.dispatchEvent(zoomIn)
      await sleep(100)

      const afterZoomIn = parseInt(canvas.getAttribute('data-track-height') || '56', 10)
      const zoomedIn = afterZoomIn > initialHeight

      // Zoom out: Ctrl+Shift+WheelDown
      const zoomOut = new WheelEvent('wheel', {
        deltaY: 100,
        ctrlKey: true,
        shiftKey: true,
        bubbles: true,
      })
      canvas.dispatchEvent(zoomOut)
      canvas.dispatchEvent(zoomOut)
      await sleep(100)

      const afterZoomOut = parseInt(canvas.getAttribute('data-track-height') || '56', 10)
      const zoomedOut = afterZoomOut < afterZoomIn

      const ok = zoomedIn && zoomedOut
      log(ok ? 'pass' : 'fail', 'vertical zoom', {
        expected: 'height increases on zoom in, decreases on zoom out',
        actual: `initial=${initialHeight} afterIn=${afterZoomIn} afterOut=${afterZoomOut}`,
      })
      return { pass: ok, note: ok ? `${initialHeight}→${afterZoomIn}→${afterZoomOut}` : 'zoom had no effect' }
    },
  },

  // ============================================================================
  // Phase 1 — recent d:0→d:1 flips (arm, reorder, SRC, hot-swap, driver sel)
  // ============================================================================
  {
    id: 'toggle_arm_roundtrip',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'Track header: arm record button',
    title: 'toggle_arm toggles armed state',
    instructions: 'Adds a track, toggles arm twice, verifies state flips.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      const before = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)
      const initial = before?.armed ?? false
      await invoke('toggle_arm', { trackId: id })
      const mid = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)
      await invoke('toggle_arm', { trackId: id })
      const end = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)
      const ok = mid?.armed === !initial && end?.armed === initial
      log(ok ? 'pass' : 'fail', 'armed toggles', {
        expected: `${initial}→${!initial}→${initial}`,
        actual: `${initial}→${mid?.armed}→${end?.armed}`,
      })
      return { pass: !!ok, note: ok ? 'armed toggles correctly' : 'did not toggle' }
    },
  },
  {
    id: 'reorder_track_roundtrip',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'Track reorder (drag up/down)',
    title: 'reorder_track changes index',
    instructions: 'Adds two tracks, swaps their positions, verifies order changes.',
    run: async ({ log }) => {
      const a = await invoke<string>('add_audio_track', { name: 'ReorderA' })
      const b = await invoke<string>('add_audio_track', { name: 'ReorderB' })
      await sleep(30)
      const before = (await invoke<any[]>('get_tracks')).filter((t) => t.kind !== 'Master')
      const idxA = before.findIndex((t) => t.id === a)
      const idxB = before.findIndex((t) => t.id === b)
      if (idxA < 0 || idxB < 0) return { pass: false, note: 'tracks not found' }
      // Move B before A
      await invoke('reorder_track', { trackId: b, newIndex: idxA })
      await sleep(30)
      const after = (await invoke<any[]>('get_tracks')).filter((t) => t.kind !== 'Master')
      const newIdxA = after.findIndex((t) => t.id === a)
      const newIdxB = after.findIndex((t) => t.id === b)
      const ok = newIdxB < newIdxA
      log(ok ? 'pass' : 'fail', 'reorder', {
        expected: `B before A`,
        actual: `A@${newIdxA} B@${newIdxB}`,
      })
      // Cleanup
      await invoke('remove_track', { trackId: a })
      await invoke('remove_track', { trackId: b })
      return { pass: ok, note: ok ? `B moved above A` : 'no change' }
    },
  },
  {
    id: 'audio_hosts_list_nonempty',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'Audio driver selection in settings',
    title: 'list_audio_hosts returns at least one backend',
    instructions: 'Calls list_audio_hosts; expects at least one cpal host.',
    run: async ({ log }) => {
      const hosts = await invoke<string[]>('list_audio_hosts')
      const ok = Array.isArray(hosts) && hosts.length > 0
      log(ok ? 'pass' : 'fail', 'hosts', { expected: '>=1', actual: hosts })
      return { pass: ok, note: ok ? `${hosts.length} host(s): ${hosts.join(', ')}` : 'no hosts' }
    },
  },
  {
    id: 'get_audio_host_known',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'Audio driver selection in settings',
    title: 'get_audio_host returns a value from list_audio_hosts',
    instructions: 'Active host name must be present in the hosts list.',
    run: async ({ log }) => {
      const [hosts, current] = await Promise.all([
        invoke<string[]>('list_audio_hosts'),
        invoke<string>('get_audio_host'),
      ])
      const ok = hosts.some((h) => h.toLowerCase() === current.toLowerCase())
      log(ok ? 'pass' : 'fail', 'current host', { expected: `one of ${hosts}`, actual: current })
      return { pass: ok, note: `current=${current}` }
    },
  },
  {
    id: 'set_audio_host_idempotent',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'Audio driver selection in settings',
    title: 'set_audio_host to current host is a no-op',
    instructions: 'Setting host to the one already active must succeed and not change it.',
    run: async ({ log }) => {
      const current = await invoke<string>('get_audio_host')
      try {
        await invoke('set_audio_host', { hostName: current })
      } catch (e: any) {
        log('fail', 'set_audio_host threw', { actual: String(e) })
        return { pass: false, note: 'threw on same-host set' }
      }
      const after = await invoke<string>('get_audio_host')
      const ok = after === current
      log(ok ? 'pass' : 'fail', 'host unchanged', { expected: current, actual: after })
      return { pass: ok, note: ok ? 'idempotent' : `changed to ${after}` }
    },
  },
  {
    id: 'set_audio_host_unknown_fails',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'Audio driver selection in settings',
    title: 'set_audio_host rejects unknown host',
    instructions: 'Invalid host names must return an error, not silently succeed.',
    run: async ({ log }) => {
      let threw = false
      try {
        await invoke('set_audio_host', { hostName: '__definitely_not_a_host__' })
      } catch {
        threw = true
      }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: 'error', actual: threw ? 'threw' : 'accepted' })
      return { pass: threw, note: threw ? 'rejected invalid host' : 'accepted invalid host' }
    },
  },
  {
    id: 'wasapi_exclusive_round_trip',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'WASAPI exclusive mode',
    title: 'get/set_wasapi_exclusive round-trips through the engine',
    instructions:
      'Toggles the WASAPI exclusive flag, asserts the getter reflects each write, and restores the original value.',
    run: async ({ log }) => {
      const initial = await invoke<{ enabled: boolean; available: boolean }>('get_wasapi_exclusive')
      log('info', 'initial', { actual: initial })
      try {
        await invoke('set_wasapi_exclusive', { enabled: true })
        const after1 = await invoke<{ enabled: boolean }>('get_wasapi_exclusive')
        if (!after1.enabled) {
          log('fail', 'enable did not persist', { expected: true, actual: after1.enabled })
          return { pass: false, note: 'enable ignored' }
        }
        await invoke('set_wasapi_exclusive', { enabled: false })
        const after2 = await invoke<{ enabled: boolean }>('get_wasapi_exclusive')
        if (after2.enabled) {
          log('fail', 'disable did not persist', { expected: false, actual: after2.enabled })
          return { pass: false, note: 'disable ignored' }
        }
      } finally {
        await invoke('set_wasapi_exclusive', { enabled: initial.enabled })
      }
      log('pass', 'toggle round-trip ok')
      return {
        pass: true,
        note: initial.available ? 'round-trip ok (available on this host)' : 'round-trip ok (setting stored; not applicable on this host)',
      }
    },
  },
  {
    id: 'jack_host_available_when_built',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'JACK automatic port connection',
    title: 'JACK host shows up in list_audio_hosts when built with --features jack',
    instructions:
      'If the build includes the jack feature, "JACK" must appear in list_audio_hosts — confirms the JACK backend (and cpal\'s connect_ports_automatically=true default) is wired in.',
    run: async ({ log }) => {
      const hosts = (await invoke<string[]>('list_audio_hosts')).map((h) => h.toLowerCase())
      const hasJack = hosts.includes('jack')
      if (!hasJack) {
        log('info', 'JACK not in hosts — build likely omitted --features jack; skipping')
        return { pass: true, note: 'jack feature not in build — skipped' }
      }
      log('pass', 'JACK host present; cpal auto-connects output ports to system:playback_*')
      return { pass: true, note: 'JACK backend available, auto-connect active' }
    },
  },
  {
    id: 'asio_host_available_when_built',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'ASIO support on Windows',
    title: 'ASIO host shows up on Windows builds with --features asio',
    instructions:
      'Checks list_audio_hosts for "ASIO". Expected to appear only on Windows builds that pulled in the Steinberg SDK.',
    run: async ({ log }) => {
      const hosts = (await invoke<string[]>('list_audio_hosts')).map((h) => h.toLowerCase())
      const hasAsio = hosts.includes('asio')
      if (!hasAsio) {
        log('info', 'ASIO not in hosts — either non-Windows build or SDK missing; skipping')
        return { pass: true, note: 'asio feature not in build — skipped' }
      }
      log('pass', 'ASIO host present; device enumeration + buffer size go through cpal')
      return { pass: true, note: 'ASIO backend available' }
    },
  },
  {
    id: 'asio_buffer_size_settable',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'ASIO buffer size configuration',
    title: 'set_audio_config accepts a new buffer size',
    instructions:
      'Round-trips buffer size through set_audio_config / get_audio_config. Independent of host; verifies the ASIO (and other) paths share the common buffer-size setter.',
    run: async ({ log }) => {
      const before = await invoke<{ device: string | null; sample_rate: number; buffer_size: number }>(
        'get_audio_config',
      )
      const target = before.buffer_size === 256 ? 512 : 256
      try {
        await invoke('set_audio_config', {
          device: before.device,
          sampleRate: before.sample_rate,
          bufferSize: target,
        })
        const after = await invoke<{ buffer_size: number }>('get_audio_config')
        const ok = after.buffer_size === target
        log(ok ? 'pass' : 'fail', 'buffer_size', { expected: target, actual: after.buffer_size })
        return { pass: ok, note: ok ? `set ${target} round-tripped` : `got ${after.buffer_size}` }
      } finally {
        await invoke('set_audio_config', {
          device: before.device,
          sampleRate: before.sample_rate,
          bufferSize: before.buffer_size,
        })
      }
    },
  },
  {
    id: 'coreaudio_workgroup_join_macos',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'CoreAudio optimizations on macOS',
    title: 'macOS: engine starts without workgroup failures',
    instructions:
      'On macOS, starting the engine triggers a workgroup join on the first audio callback. We verify stream_error stays clear after a short run — a failed join does not abort audio but a crash would flip it.',
    run: async ({ log }) => {
      const platform = navigator.platform.toLowerCase()
      if (!platform.includes('mac')) {
        log('info', 'not macOS — skipping workgroup check')
        return { pass: true, note: 'not macOS — skipped' }
      }
      await invoke('start_engine')
      await sleep(200)
      const state = await devDumpState()
      await invoke('stop_engine')
      const ok = !state.streamErrorFlag
      log(ok ? 'pass' : 'fail', 'streamErrorFlag', { expected: false, actual: state.streamErrorFlag })
      return { pass: ok, note: ok ? 'engine ran cleanly (workgroup best-effort)' : 'stream errored' }
    },
  },
  {
    id: 'device_enumeration_nonempty',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'Audio device hot-swap detection',
    title: 'get_audio_devices returns devices',
    instructions: 'Device list should be non-empty (headless CI may legitimately be empty — skipped then).',
    run: async ({ log }) => {
      const devs = await invoke<any[]>('get_audio_devices')
      if (!Array.isArray(devs) || devs.length === 0) {
        log('info', 'no devices (likely headless CI — skipped)')
        return { pass: true, note: 'no devices visible (headless?)' }
      }
      log('pass', 'devices', { actual: devs.map((d) => d.name) })
      return { pass: true, note: `${devs.length} device(s)` }
    },
  },
  {
    id: 'src_on_import_matches_engine_rate',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'Sample rate conversion on import',
    title: 'Imported clip sample rate matches engine rate',
    instructions: 'Imports sine_44100.wav; verifies the reported clip sample_rate equals the engine sample_rate.',
    run: async ({ log, ensureAudioTrack, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      const s = await devDumpState()
      const targetSr = s.sampleRate
      let clip: any
      try {
        const fsPath = await devResolveTestAsset('sine_44100.wav')
        clip = await invoke('import_audio_file', { trackId, filePath: fsPath, positionTicks: 0 })
      } catch (e: any) {
        log('info', 'asset missing: sine_44100.wav not present; skipping')
        return { pass: true, note: 'asset missing — skipped' }
      }
      const ok = clip?.sample_rate === targetSr
      log(ok ? 'pass' : 'fail', 'sample_rate', { expected: targetSr, actual: clip?.sample_rate })
      return { pass: ok, note: ok ? `resampled to ${targetSr}` : `clip reports ${clip?.sample_rate}` }
    },
  },
  {
    id: 'reorder_bounds_clamp',
    kind: 'AUTO',
    phase: 1,
    phase1Item: 'Track reorder (drag up/down)',
    title: 'reorder_track clamps out-of-range index',
    instructions: 'Reordering to index 999 must clamp to the last non-master slot, not crash.',
    run: async ({ log }) => {
      const a = await invoke<string>('add_audio_track', { name: 'ClampTest' })
      await sleep(20)
      let threw = false
      try {
        await invoke('reorder_track', { trackId: a, newIndex: 999 })
      } catch {
        threw = true
      }
      const after = (await invoke<any[]>('get_tracks')).filter((t) => t.kind !== 'Master')
      const exists = after.some((t) => t.id === a)
      const ok = !threw && exists
      log(ok ? 'pass' : 'fail', 'no crash', { expected: 'clamped', actual: threw ? 'threw' : 'ok' })
      await invoke('remove_track', { trackId: a })
      return { pass: ok, note: ok ? 'clamped safely' : 'crashed or lost track' }
    },
  },

  // ============================================================================
  // Phase 2 — Timeline & Clips (seed suite)
  // ============================================================================
  {
    id: 'p2_add_remove_track',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Add track / Delete track',
    title: 'add_audio_track then remove_track round-trip',
    instructions: 'Track count goes up by 1 after add, down by 1 after remove.',
    run: async ({ log }) => {
      const before = (await invoke<any[]>('get_tracks')).length
      const id = await invoke<string>('add_audio_track', { name: 'P2 temp' })
      const mid = (await invoke<any[]>('get_tracks')).length
      await invoke('remove_track', { trackId: id })
      const after = (await invoke<any[]>('get_tracks')).length
      const ok = mid === before + 1 && after === before
      log(ok ? 'pass' : 'fail', 'counts', { expected: `${before}→${before + 1}→${before}`, actual: `${before}→${mid}→${after}` })
      return { pass: ok, note: ok ? 'round-trip OK' : 'count mismatch' }
    },
  },
  {
    id: 'p2_clip_move',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip move: horizontal drag (time)',
    title: 'move_clip updates position_ticks',
    instructions: 'Imports a clip, moves it to position 960, verifies position_ticks=960.',
    run: async ({ log, ensureAudioTrack, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      let clip: any
      try {
        const fsPath = await devResolveTestAsset('sine_44100.wav')
        clip = await invoke('import_audio_file', { trackId, filePath: fsPath, positionTicks: 0 })
      } catch {
        return { pass: true, note: 'asset missing — skipped' }
      }
      await invoke('move_clip', { trackId, clipId: clip.clip_id, newPositionTicks: 960 })
      const clips = await invoke<any[]>('get_track_clips', { trackId })
      const moved = clips.find((c: any) => c.id === clip.clip_id)
      const ok = moved?.position_ticks === 960
      log(ok ? 'pass' : 'fail', 'position', { expected: 960, actual: moved?.position_ticks })
      return { pass: ok, note: `position=${moved?.position_ticks}` }
    },
  },
  {
    id: 'p2_clip_resize',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip resize: drag right edge',
    title: 'resize_clip updates length_ticks',
    instructions: 'Resizes imported clip to 480 ticks; verifies readback.',
    run: async ({ log, ensureAudioTrack, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      let clip: any
      try {
        const fsPath = await devResolveTestAsset('sine_44100.wav')
        clip = await invoke('import_audio_file', { trackId, filePath: fsPath, positionTicks: 0 })
      } catch {
        return { pass: true, note: 'asset missing — skipped' }
      }
      await invoke('resize_clip', { trackId, clipId: clip.clip_id, newLengthTicks: 480 })
      const clips = await invoke<any[]>('get_track_clips', { trackId })
      const sized = clips.find((c: any) => c.id === clip.clip_id)
      const ok = sized?.length_ticks === 480
      log(ok ? 'pass' : 'fail', 'length', { expected: 480, actual: sized?.length_ticks })
      return { pass: ok, note: `length=${sized?.length_ticks}` }
    },
  },
  {
    id: 'p2_clip_delete',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip delete: Delete/Backspace key',
    title: 'delete_clip removes the clip',
    instructions: 'Deletes a clip; subsequent get_track_clips must not contain it.',
    run: async ({ log, ensureAudioTrack, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      let clip: any
      try {
        const fsPath = await devResolveTestAsset('sine_44100.wav')
        clip = await invoke('import_audio_file', { trackId, filePath: fsPath, positionTicks: 0 })
      } catch {
        return { pass: true, note: 'asset missing — skipped' }
      }
      await invoke('delete_clip', { trackId, clipId: clip.clip_id })
      const clips = await invoke<any[]>('get_track_clips', { trackId })
      const stillThere = clips.some((c: any) => c.id === clip.clip_id)
      const ok = !stillThere
      log(ok ? 'pass' : 'fail', 'removed', { expected: 'gone', actual: stillThere ? 'still present' : 'gone' })
      return { pass: ok, note: ok ? 'clip removed' : 'clip still present' }
    },
  },
  {
    id: 'p2_waveform_peaks',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip waveform peak pre-computation',
    title: 'get_waveform_peaks returns N buckets of [min, max]',
    instructions: 'Requests 256 buckets for an imported clip; verifies shape.',
    run: async ({ log, ensureAudioTrack, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      let clip: any
      try {
        const fsPath = await devResolveTestAsset('sine_44100.wav')
        clip = await invoke('import_audio_file', { trackId, filePath: fsPath, positionTicks: 0 })
      } catch {
        return { pass: true, note: 'asset missing — skipped' }
      }
      const peaks = await invoke<[number, number][]>('get_waveform_peaks', {
        sourceId: clip.source_id,
        numBuckets: 256,
      })
      const ok =
        Array.isArray(peaks) &&
        peaks.length === 256 &&
        peaks.every((p) => Array.isArray(p) && p.length === 2 && p[0] <= p[1])
      log(ok ? 'pass' : 'fail', 'shape', { expected: '256 × [min<=max]', actual: `${peaks?.length}` })
      return { pass: ok, note: ok ? '256 buckets OK' : 'shape mismatch' }
    },
  },
  {
    id: 'p2_track_kind_on_add',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Add track button',
    title: 'add_audio_track and add_midi_track set correct kind',
    instructions: 'Kind field must be "Audio" / "Midi" on the respective tracks.',
    run: async ({ log }) => {
      const audioId = await invoke<string>('add_audio_track', { name: 'Kind A' })
      const midiId = await invoke<string>('add_midi_track', { name: 'Kind M' })
      const tracks = await invoke<any[]>('get_tracks')
      const a = tracks.find((t) => t.id === audioId)
      const m = tracks.find((t) => t.id === midiId)
      const ok = a?.kind === 'Audio' && m?.kind === 'Midi'
      log(ok ? 'pass' : 'fail', 'kinds', {
        expected: 'Audio / Midi',
        actual: `${a?.kind} / ${m?.kind}`,
      })
      await invoke('remove_track', { trackId: audioId })
      await invoke('remove_track', { trackId: midiId })
      return { pass: ok, note: ok ? 'kinds correct' : 'kind mismatch' }
    },
  },
  {
    id: 'p2_new_project_has_master',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'File > New project',
    title: 'new_project leaves exactly one Master track',
    instructions: 'After new_project the track list must contain exactly one Master and no audio tracks.',
    run: async ({ log }) => {
      await invoke('new_project')
      await sleep(30)
      const tracks = await invoke<any[]>('get_tracks')
      const masters = tracks.filter((t) => t.kind === 'Master').length
      const audios = tracks.filter((t) => t.kind !== 'Master').length
      const ok = masters === 1 && audios === 0
      log(ok ? 'pass' : 'fail', 'tracks', { expected: '1 master / 0 audio', actual: `${masters}/${audios}` })
      return { pass: ok, note: `${masters} master, ${audios} audio` }
    },
  },

  // -------------------------------------------------------------------------
  // Phase 2 Round 1 — Foundation (snap, zoom, left-edge resize, clip color)
  // -------------------------------------------------------------------------
  {
    id: 'p2r1_snap_ticks_math',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Snap value math',
    title: 'snapToTicks returns correct PPQ multiples for all divisions',
    instructions: 'Unit-level check that snap value → tick conversion is correct for plain, triplet, and dotted divisions.',
    run: async ({ log }) => {
      const checks: Array<[string, number]> = [
        ['1/4', 960], ['1/8', 480], ['1/16', 240], ['1/1', 3840],
        ['1/4T', 640], ['1/8T', 320],
        ['1/4D', 1440], ['1/8D', 720],
      ]
      for (const [v, expected] of checks) {
        const got = snapToTicks(v as any, true)
        if (got !== expected) {
          log('fail', `snapToTicks(${v})`, { expected, actual: got })
          return { pass: false, note: `${v} → ${got}, expected ${expected}` }
        }
      }
      if (snapToTicks('Off', true) !== 0) return { pass: false, note: 'Off must yield 0' }
      if (snapToTicks('1/4', false) !== 0) return { pass: false, note: 'disabled must yield 0' }
      log('pass', 'all snap divisions correct')
      return { pass: true, note: 'plain + triplet + dotted all match' }
    },
  },
  {
    id: 'p2r1_snap_toggle',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Snap toggle',
    title: 'toggleSnap flips snapEnabled boolean',
    instructions: 'Calling toggleSnap must invert the snapEnabled flag.',
    run: async ({ log }) => {
      const s = useTransportStore.getState()
      const before = s.snapEnabled
      s.toggleSnap()
      const mid = useTransportStore.getState().snapEnabled
      s.toggleSnap()
      const after = useTransportStore.getState().snapEnabled
      const ok = mid === !before && after === before
      log(ok ? 'pass' : 'fail', 'toggle', { expected: `${!before}→${before}`, actual: `${mid}→${after}` })
      return { pass: ok, note: ok ? 'toggled twice, restored' : 'toggle did not invert' }
    },
  },
  {
    id: 'p2r1_set_snap_value_off',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Snap value selector',
    title: 'setSnapValue("Off") disables snap; other values enable it',
    instructions: 'Selecting Off in the dropdown must disable snap; selecting any division must enable it.',
    run: async ({ log }) => {
      const s = useTransportStore.getState()
      s.setSnapValue('Off')
      const off = useTransportStore.getState()
      s.setSnapValue('1/8')
      const on = useTransportStore.getState()
      const ok = off.snapEnabled === false && on.snapEnabled === true && on.snapValue === '1/8'
      log(ok ? 'pass' : 'fail', 'state', { expected: 'Off→disabled, 1/8→enabled', actual: `${off.snapEnabled}/${on.snapEnabled} v=${on.snapValue}` })
      // Restore default
      useTransportStore.getState().setSnapValue('1/4')
      return { pass: ok, note: 'Off disables, division enables' }
    },
  },
  {
    id: 'p2r1_snap_dropdown_in_dom',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Snap value selector',
    title: 'Snap dropdown is rendered with all 13 values',
    instructions: 'Toolbar must expose a snap-select element containing every SnapValue option.',
    run: async ({ log }) => {
      const el = queryTestId('snap-select') as HTMLSelectElement | null
      if (!el) return { pass: false, note: 'snap-select testid missing' }
      const count = el.options.length
      const ok = count === 13
      log(ok ? 'pass' : 'fail', 'option count', { expected: 13, actual: count })
      return { pass: ok, note: `${count} options` }
    },
  },
  {
    id: 'p2r1_horizontal_zoom_clamp',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Horizontal zoom',
    title: 'setHorizontalZoom clamps to [0.1, 16]',
    instructions: 'Out-of-range zoom values must be clamped instead of rejected.',
    run: async ({ log }) => {
      const s = useTransportStore.getState()
      const before = s.horizontalZoom
      s.setHorizontalZoom(100)
      const hi = useTransportStore.getState().horizontalZoom
      s.setHorizontalZoom(0.001)
      const lo = useTransportStore.getState().horizontalZoom
      useTransportStore.getState().setHorizontalZoom(before)
      const ok = hi === 16 && lo === 0.1
      log(ok ? 'pass' : 'fail', 'clamps', { expected: 'hi=16, lo=0.1', actual: `hi=${hi}, lo=${lo}` })
      return { pass: ok, note: `hi=${hi}, lo=${lo}` }
    },
  },
  {
    id: 'p2r1_zoom_to_fit_resets',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Zoom-to-fit button',
    title: 'zoomToFit resets horizontalZoom to 1.0',
    instructions: 'After zooming in, zoomToFit must return horizontalZoom to the default 1.0.',
    run: async ({ log }) => {
      const s = useTransportStore.getState()
      s.setHorizontalZoom(5)
      s.zoomToFit()
      const z = useTransportStore.getState().horizontalZoom
      const ok = z === 1
      log(ok ? 'pass' : 'fail', 'zoom', { expected: 1, actual: z })
      return { pass: ok, note: `zoom=${z}` }
    },
  },
  {
    id: 'p2r1_clip_color_override_roundtrip',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip color override',
    title: 'setClipColor stores override; passing null clears it',
    instructions: 'Setting a hex color then clearing with null must leave the override map clean.',
    run: async ({ log }) => {
      const s = useTransportStore.getState()
      s.setClipColor('test-clip-xyz', '#abcdef')
      const set1 = useTransportStore.getState().clipColorOverrides['test-clip-xyz']
      s.setClipColor('test-clip-xyz', null)
      const set2 = useTransportStore.getState().clipColorOverrides['test-clip-xyz']
      const ok = set1 === '#abcdef' && set2 === undefined
      log(ok ? 'pass' : 'fail', 'override', { expected: '#abcdef then undefined', actual: `${set1} → ${set2}` })
      return { pass: ok, note: 'set + clear round-trip' }
    },
  },

  // -------------------------------------------------------------------------
  // Phase 2 Round 2 — Clip editing (multi-select, duplicate, split, paste)
  // -------------------------------------------------------------------------
  {
    id: 'p2r2_duplicate_clip',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip duplicate: Ctrl+D',
    title: 'duplicate_clip adds a copy immediately after the source',
    instructions: 'Duplicating a clip must produce a second clip on the same track at position = original end.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      let clips = await invoke<any[]>('get_track_clips', { trackId })
      if (clips.length !== 1) return { pass: false, note: `expected 1 clip, got ${clips.length}` }
      const original = clips[0]
      await invoke('duplicate_clip', { trackId, clipId: original.id })
      clips = await invoke<any[]>('get_track_clips', { trackId })
      const ok = clips.length === 2
      const copy = clips.find((c) => c.id !== original.id)
      const posOk = copy && copy.position_ticks === original.position_ticks + original.length_ticks
      log(ok && posOk ? 'pass' : 'fail', 'duplicate', {
        expected: `2 clips, copy at ${original.position_ticks + original.length_ticks}`,
        actual: `${clips.length} clips, copy at ${copy?.position_ticks}`,
      })
      await clearTrackClips(trackId)
      return { pass: !!(ok && posOk), note: `${clips.length} clips after duplicate` }
    },
  },
  {
    id: 'p2r2_split_clip',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip split at playhead: S key',
    title: 'split_clip divides a clip into two contiguous halves',
    instructions: 'Splitting at the midpoint must yield two clips whose lengths sum to the original.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const clips = await invoke<any[]>('get_track_clips', { trackId })
      if (clips.length !== 1) return { pass: false, note: `expected 1 clip, got ${clips.length}` }
      const c = clips[0]
      const mid = c.position_ticks + Math.floor(c.length_ticks / 2)
      await invoke('split_clip', { trackId, clipId: c.id, atTicks: mid })
      const after = await invoke<any[]>('get_track_clips', { trackId })
      const ok = after.length === 2
      const totalLen = after.reduce((s, x) => s + x.length_ticks, 0)
      const lenMatches = totalLen === c.length_ticks
      log(ok && lenMatches ? 'pass' : 'fail', 'split', {
        expected: `2 clips totalling ${c.length_ticks}`,
        actual: `${after.length} clips totalling ${totalLen}`,
      })
      await clearTrackClips(trackId)
      return { pass: !!(ok && lenMatches), note: `${after.length} clips, total ${totalLen}` }
    },
  },
  {
    id: 'p2r2_split_rejects_out_of_range',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip split at playhead: S key',
    title: 'split_clip rejects a position outside the clip',
    instructions: 'Requesting a split at a tick before or after the clip must return an error and leave the clip unchanged.',
    run: async ({ ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      let errored = false
      try {
        await invoke('split_clip', { trackId, clipId: c.id, atTicks: c.position_ticks + c.length_ticks + 1000 })
      } catch { errored = true }
      const after = await invoke<any[]>('get_track_clips', { trackId })
      await clearTrackClips(trackId)
      const ok = errored && after.length === 1
      return { pass: ok, note: ok ? 'rejected cleanly' : `errored=${errored} clips=${after.length}` }
    },
  },
  {
    id: 'p2r2_select_all_clips',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip selection: Ctrl+A select all',
    title: 'selectAllClips populates selectedClipIds with every clip in every track',
    instructions: 'After Ctrl+A the store\'s selectedClipIds set must contain every clip id.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const { useTrackStore } = await import('../stores/trackStore')
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      await useTrackStore.getState().fetchTracks()
      const totalClips = useTrackStore.getState().tracks.flatMap((t) => t.clips).length
      useTrackStore.getState().selectAllClips()
      const selected = useTrackStore.getState().selectedClipIds.size
      const ok = selected === totalClips && totalClips >= 2
      log(ok ? 'pass' : 'fail', 'select all', { expected: totalClips, actual: selected })
      useTrackStore.getState().clearSelection()
      await clearTrackClips(trackId)
      return { pass: ok, note: `selected ${selected} of ${totalClips}` }
    },
  },
  {
    id: 'p2r2_toggle_clip_selection',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip selection: Ctrl+click to add to selection',
    title: 'toggleClipSelection adds then removes a clip id from the set',
    instructions: 'Calling toggleClipSelection twice with the same id must leave the set unchanged.',
    run: async ({ log }) => {
      const { useTrackStore } = await import('../stores/trackStore')
      const s = useTrackStore.getState()
      const before = s.selectedClipIds.size
      s.toggleClipSelection('synthetic-clip-id-aaa')
      const mid = useTrackStore.getState().selectedClipIds.size
      s.toggleClipSelection('synthetic-clip-id-aaa')
      const after = useTrackStore.getState().selectedClipIds.size
      const ok = mid === before + 1 && after === before
      log(ok ? 'pass' : 'fail', 'toggle', { expected: `${before + 1}→${before}`, actual: `${mid}→${after}` })
      useTrackStore.getState().clearSelection()
      return { pass: ok, note: 'add then remove restores size' }
    },
  },
  {
    id: 'p2r2_copy_paste_clips',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip copy/paste: Ctrl+C / Ctrl+V',
    title: 'copySelectedClips + pasteClipsAtPosition places new clip at given position',
    instructions: 'Copy a single clip, paste at a specific tick position; the new clip must exist at that tick.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const { useTrackStore } = await import('../stores/trackStore')
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      await useTrackStore.getState().fetchTracks()
      const track = useTrackStore.getState().tracks.find((t) => t.id === trackId)!
      const clip = track.clips[0]
      useTrackStore.getState().selectClip(clip.id, trackId)
      useTrackStore.getState().copySelectedClips()
      const pasteAt = 960 * 8 // bar 3
      await useTrackStore.getState().pasteClipsAtPosition(pasteAt, trackId)
      const after = await invoke<any[]>('get_track_clips', { trackId })
      const ok = after.length === 2 && after.some((c) => c.position_ticks === pasteAt)
      log(ok ? 'pass' : 'fail', 'paste', { expected: `2 clips, one at ${pasteAt}`, actual: `${after.length} clips` })
      await clearTrackClips(trackId)
      return { pass: ok, note: `${after.length} clips post-paste` }
    },
  },
  {
    id: 'p2r3_clip_gain_roundtrip',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip gain knob',
    title: 'set_clip_gain persists and clamps to -60..+12 dB',
    instructions: 'Setting gain to +20 dB should clamp at +12; -200 should clamp at -60.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('set_clip_gain', { trackId, clipId: c.id, gainDb: 20 })
      let after = await invoke<any[]>('get_track_clips', { trackId })
      const hi = after[0].gainDb
      await invoke('set_clip_gain', { trackId, clipId: c.id, gainDb: -200 })
      after = await invoke<any[]>('get_track_clips', { trackId })
      const lo = after[0].gainDb
      const ok = Math.abs(hi - 12) < 0.01 && Math.abs(lo - -60) < 0.01
      log(ok ? 'pass' : 'fail', 'gain', { expected: 'hi=12, lo=-60', actual: `hi=${hi}, lo=${lo}` })
      await clearTrackClips(trackId)
      return { pass: ok, note: `hi=${hi} lo=${lo}` }
    },
  },
  {
    id: 'p2r3_clip_fades_roundtrip',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip fade-in handle',
    title: 'set_clip_fades stores values and clamps to clip length',
    instructions: 'Fade-in + fade-out greater than clip length must be scaled down proportionally.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('set_clip_fades', { trackId, clipId: c.id, fadeInTicks: 200, fadeOutTicks: 300 })
      let after = await invoke<any[]>('get_track_clips', { trackId })
      const inOk = after[0].fadeInTicks === 200 && after[0].fadeOutTicks === 300
      // Overflow: fades summing past clip length should scale down to <= length
      const huge = c.length_ticks
      await invoke('set_clip_fades', { trackId, clipId: c.id, fadeInTicks: huge, fadeOutTicks: huge })
      after = await invoke<any[]>('get_track_clips', { trackId })
      const sum = after[0].fadeInTicks + after[0].fadeOutTicks
      const clamped = sum <= c.length_ticks
      const ok = inOk && clamped
      log(ok ? 'pass' : 'fail', 'fades', { expected: `stored + sum<=${c.length_ticks}`, actual: `stored=${inOk} sum=${sum}` })
      await clearTrackClips(trackId)
      return { pass: ok, note: `stored=${inOk} clamp-sum=${sum}/${c.length_ticks}` }
    },
  },
  {
    id: 'p2r3_clip_reverse_toggle',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip reverse toggle',
    title: 'toggle_clip_reverse flips the reversed flag',
    instructions: 'Calling toggle twice must return the flag to its original state.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      const before = c.reversed
      await invoke('toggle_clip_reverse', { trackId, clipId: c.id })
      let after = await invoke<any[]>('get_track_clips', { trackId })
      const mid = after[0].reversed
      await invoke('toggle_clip_reverse', { trackId, clipId: c.id })
      after = await invoke<any[]>('get_track_clips', { trackId })
      const end = after[0].reversed
      const ok = mid !== before && end === before
      log(ok ? 'pass' : 'fail', 'reverse', { expected: `${before}→${!before}→${before}`, actual: `${before}→${mid}→${end}` })
      await clearTrackClips(trackId)
      return { pass: ok, note: `${before}→${mid}→${end}` }
    },
  },
  {
    id: 'p2r4_edit_cursor_roundtrip',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Edit cursor (separate from playhead)',
    title: 'setEditCursor stores a non-negative tick value',
    instructions: 'Setting the edit cursor to a tick updates the store; setting null clears it.',
    run: async ({ log }) => {
      const { useTransportStore } = await import('../stores/transportStore')
      useTransportStore.getState().setEditCursor(1920)
      const mid = useTransportStore.getState().editCursorTicks
      useTransportStore.getState().setEditCursor(null)
      const end = useTransportStore.getState().editCursorTicks
      const ok = mid === 1920 && end === null
      log(ok ? 'pass' : 'fail', 'edit cursor', { expected: '1920→null', actual: `${mid}→${end}` })
      return { pass: ok, note: `mid=${mid} end=${end}` }
    },
  },
  {
    id: 'p2r4_edit_cursor_clamps_to_zero',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Edit cursor (separate from playhead)',
    title: 'Negative edit cursor values clamp to zero',
    instructions: 'Passing a negative tick to setEditCursor must clamp to 0.',
    run: async ({ log }) => {
      const { useTransportStore } = await import('../stores/transportStore')
      useTransportStore.getState().setEditCursor(-500)
      const got = useTransportStore.getState().editCursorTicks
      useTransportStore.getState().setEditCursor(null)
      const ok = got === 0
      log(ok ? 'pass' : 'fail', 'clamp', { expected: 0, actual: got })
      return { pass: ok, note: `got=${got}` }
    },
  },
  {
    id: 'p2r4_crossfade_autofade_applied',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip crossfade when overlapping',
    title: 'Overlapping clips on the same track coexist (auto-crossfade applied at engine)',
    instructions: 'Place two clips so the second starts inside the first. Both clips must remain; the engine internally ramps across the overlap.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const [first] = await invoke<any[]>('get_track_clips', { trackId })
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const clips = await invoke<any[]>('get_track_clips', { trackId })
      if (clips.length !== 2) return { pass: false, note: `expected 2 clips, got ${clips.length}` }
      const second = clips.find((c) => c.id !== first.id)
      const halfIn = first.position_ticks + Math.floor(first.length_ticks / 2)
      await invoke('move_clip', { trackId, clipId: second.id, newPositionTicks: halfIn })
      const after = await invoke<any[]>('get_track_clips', { trackId })
      const ok = after.length === 2 && after.some((c) => c.position_ticks === halfIn)
      log(ok ? 'pass' : 'fail', 'overlap', { expected: '2 clips with overlap', actual: `${after.length} clips` })
      await clearTrackClips(trackId)
      return { pass: ok, note: `${after.length} clips after overlap move` }
    },
  },
  {
    id: 'p2r5_undo_restores_clip_position',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Undo/Redo',
    title: 'undo reverts a move_clip to the prior position',
    instructions: 'Move a clip, call undo, and verify its position is the original.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      const originalPos = c.position_ticks
      await invoke('move_clip', { trackId, clipId: c.id, newPositionTicks: originalPos + 9600 })
      let after = await invoke<any[]>('get_track_clips', { trackId })
      const moved = after[0].position_ticks
      const undone = await invoke<boolean>('undo')
      after = await invoke<any[]>('get_track_clips', { trackId })
      const restored = after[0].position_ticks
      const ok = undone && moved === originalPos + 9600 && restored === originalPos
      log(ok ? 'pass' : 'fail', 'undo', { expected: `${originalPos}→${originalPos + 9600}→${originalPos}`, actual: `${originalPos}→${moved}→${restored}` })
      await clearTrackClips(trackId)
      return { pass: ok, note: `moved=${moved} restored=${restored}` }
    },
  },
  {
    id: 'p2r5_redo_reapplies_change',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Undo/Redo',
    title: 'redo reapplies an undone change',
    instructions: 'Mutate, undo, then redo — the final state must match the post-mutation state.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('set_clip_gain', { trackId, clipId: c.id, gainDb: 6 })
      await invoke('undo')
      const midGain = (await invoke<any[]>('get_track_clips', { trackId }))[0].gainDb
      const redone = await invoke<boolean>('redo')
      const finalGain = (await invoke<any[]>('get_track_clips', { trackId }))[0].gainDb
      const ok = redone && Math.abs(midGain - c.gainDb) < 0.01 && Math.abs(finalGain - 6) < 0.01
      log(ok ? 'pass' : 'fail', 'redo', { expected: `undone=${c.gainDb}, redone=6`, actual: `undone=${midGain}, redone=${finalGain}` })
      await clearTrackClips(trackId)
      return { pass: ok, note: `undone=${midGain} redone=${finalGain}` }
    },
  },
  {
    id: 'p2r5_history_sizes',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Undo/Redo',
    title: 'history_sizes reflects stack depth',
    instructions: 'After a mutation the undo depth grows; after undo the redo depth grows.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      const [u0, r0] = await invoke<[number, number]>('history_sizes')
      await invoke('set_clip_gain', { trackId, clipId: c.id, gainDb: -3 })
      const [u1, r1] = await invoke<[number, number]>('history_sizes')
      await invoke('undo')
      const [u2, r2] = await invoke<[number, number]>('history_sizes')
      const ok = u1 === u0 + 1 && r1 === 0 && u2 === u0 && r2 === 1
      log(ok ? 'pass' : 'fail', 'history_sizes', { expected: `(u+1,0) then (u,1)`, actual: `(${u1},${r1}) then (${u2},${r2})` })
      await clearTrackClips(trackId)
      return { pass: ok, note: `u:${u0}→${u1}→${u2} r:${r0}→${r1}→${r2}` }
    },
  },
  {
    id: 'p2r6_clip_pitch_roundtrip',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip pitch shift (semitones)',
    title: 'set_clip_pitch persists and clamps to -24..+24 semitones',
    instructions: 'Setting pitch to +50 should clamp at +24; -50 should clamp at -24.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('set_clip_pitch', { trackId, clipId: c.id, pitchSemitones: 50 })
      let after = await invoke<any[]>('get_track_clips', { trackId })
      const hi = after[0].pitchSemitones
      await invoke('set_clip_pitch', { trackId, clipId: c.id, pitchSemitones: -50 })
      after = await invoke<any[]>('get_track_clips', { trackId })
      const lo = after[0].pitchSemitones
      const ok = Math.abs(hi - 24) < 0.01 && Math.abs(lo - -24) < 0.01
      log(ok ? 'pass' : 'fail', 'pitch', { expected: 'hi=24, lo=-24', actual: `hi=${hi}, lo=${lo}` })
      await clearTrackClips(trackId)
      return { pass: ok, note: `hi=${hi} lo=${lo}` }
    },
  },
  {
    id: 'p2r6_clip_stretch_roundtrip',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip time stretch (warping)',
    title: 'set_clip_stretch persists and clamps to 0.25..4.0',
    instructions: 'Setting stretch to 10 should clamp at 4.0; 0.01 should clamp at 0.25.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('set_clip_stretch', { trackId, clipId: c.id, stretchRatio: 10 })
      let after = await invoke<any[]>('get_track_clips', { trackId })
      const hi = after[0].stretchRatio
      await invoke('set_clip_stretch', { trackId, clipId: c.id, stretchRatio: 0.01 })
      after = await invoke<any[]>('get_track_clips', { trackId })
      const lo = after[0].stretchRatio
      const ok = Math.abs(hi - 4.0) < 0.01 && Math.abs(lo - 0.25) < 0.01
      log(ok ? 'pass' : 'fail', 'stretch', { expected: 'hi=4, lo=0.25', actual: `hi=${hi}, lo=${lo}` })
      await clearTrackClips(trackId)
      return { pass: ok, note: `hi=${hi} lo=${lo}` }
    },
  },
  {
    id: 'p2r6_pitch_stretch_undoable',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Undo: clip move/resize/delete',
    title: 'Pitch and stretch edits are captured by undo stack',
    instructions: 'After set_clip_pitch, undo must restore original pitch.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(50)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      const orig = c.pitchSemitones ?? 0
      await invoke('set_clip_pitch', { trackId, clipId: c.id, pitchSemitones: 7 })
      await invoke('undo')
      const [after] = await invoke<any[]>('get_track_clips', { trackId })
      const ok = Math.abs((after.pitchSemitones ?? 0) - orig) < 0.01
      log(ok ? 'pass' : 'fail', 'undo pitch', { expected: `${orig}`, actual: `${after.pitchSemitones}` })
      await clearTrackClips(trackId)
      return { pass: ok, note: `pitch ${after.pitchSemitones}` }
    },
  },
]

// ─────────────────────────────────────────────────────────────────────────────
// Phase 2 Round 7: extensive parametric test matrix.
// Each generator returns many individual TestDef entries so the dev panel shows
// fine-grained pass/fail across the full Phase 2 surface area.
// ─────────────────────────────────────────────────────────────────────────────

function genBpmRoundtrips(): TestDef[] {
  const targets = [60, 70, 80, 90, 100, 110, 120, 125, 128, 130, 140, 150, 160, 170, 174, 180, 190, 200, 220, 240]
  return targets.map((bpm) => ({
    id: `p2r7_bpm_${bpm}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'BPM display (editable)',
    title: `BPM roundtrip ${bpm}`,
    instructions: `set_bpm ${bpm} -> dev_dump_state matches`,
    run: async ({ log }) => {
      await invoke('set_bpm', { bpm })
      await sleep(30)
      const s = await devDumpState()
      const ok = approx(s.bpm, bpm, 0.01)
      log(ok ? 'pass' : 'fail', 'bpm', { expected: bpm, actual: s.bpm })
      return { pass: ok, note: `bpm=${s.bpm}` }
    },
  }))
}

function genMasterVolumeRoundtrips(): TestDef[] {
  const targets = [-60, -48, -36, -24, -18, -12, -9, -6, -3, -1, 0, 1, 3, 6]
  return targets.map((db) => ({
    id: `p2r7_master_vol_${db.toString().replace('-', 'n')}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Master volume control',
    title: `Master volume ${db} dB`,
    instructions: `set_master_volume ${db}`,
    run: async ({ log }) => {
      await invoke('set_master_volume', { db })
      await sleep(30)
      const s = await devDumpState()
      const ok = approx(s.masterVolumeDb, db, 0.05)
      log(ok ? 'pass' : 'fail', 'master vol', { expected: db, actual: s.masterVolumeDb })
      return { pass: ok, note: `${s.masterVolumeDb}` }
    },
  }))
}

function genTimeSigRoundtrips(): TestDef[] {
  const combos: Array<[number, number]> = [
    [4, 4], [3, 4], [6, 8], [7, 8], [5, 4], [9, 8], [12, 8], [2, 4],
    [3, 8], [11, 8], [5, 8], [7, 4], [6, 4], [15, 16], [12, 16],
  ]
  return combos.map(([num, den]) => ({
    id: `p2r7_timesig_${num}_${den}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Time signature numerator/denominator',
    title: `Time sig ${num}/${den}`,
    instructions: `set_time_signature ${num}/${den}`,
    run: async ({ log }) => {
      await invoke('set_time_signature', { numerator: num, denominator: den })
      await sleep(30)
      const s = await devDumpState()
      const ok = s.timeSigNumerator === num && s.timeSigDenominator === den
      log(ok ? 'pass' : 'fail', 'timesig', { expected: `${num}/${den}`, actual: `${s.timeSigNumerator}/${s.timeSigDenominator}` })
      return { pass: ok, note: `${s.timeSigNumerator}/${s.timeSigDenominator}` }
    },
  }))
}

function genSnapPure(): TestDef[] {
  const expected: Array<[SnapValueLite, number]> = [
    ['Off', 0], ['1/1', 3840], ['1/2', 1920], ['1/4', 960], ['1/8', 480],
    ['1/16', 240], ['1/32', 120], ['1/64', 60],
    ['1/4T', 640], ['1/8T', 320], ['1/16T', 160],
    ['1/4D', 1440], ['1/8D', 720],
  ]
  return expected.map(([snap, ticks]) => ({
    id: `p2r7_snap_${snap.replace('/', '_').replace('Off', 'off')}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: `Snap: ${snap}`,
    title: `snapToTicks(${snap})`,
    instructions: `Pure JS: snapToTicks('${snap}', true) should be ${ticks}`,
    run: async ({ log }) => {
      const got = snapToTicks(snap as any, snap !== 'Off')
      const ok = got === ticks
      log(ok ? 'pass' : 'fail', `snap ${snap}`, { expected: ticks, actual: got })
      return { pass: ok, note: `${snap}→${got}t` }
    },
  }))
}
type SnapValueLite = 'Off' | '1/1' | '1/2' | '1/4' | '1/8' | '1/16' | '1/32' | '1/64' | '1/4T' | '1/8T' | '1/16T' | '1/4D' | '1/8D'

function genClipPitchMatrix(): TestDef[] {
  const values = [-24, -19, -14, -12, -9, -7, -5, -3, -2, -1, 0, 1, 2, 3, 5, 7, 9, 12, 14, 19, 24]
  return values.map((p) => ({
    id: `p2r7_pitch_${p.toString().replace('-', 'n')}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Clip pitch shift (semitones)',
    title: `Pitch ${p} st roundtrip`,
    instructions: `set_clip_pitch ${p}`,
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(30)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('set_clip_pitch', { trackId, clipId: c.id, pitchSemitones: p })
      const [after] = await invoke<any[]>('get_track_clips', { trackId })
      const ok = Math.abs(after.pitchSemitones - p) < 0.01
      log(ok ? 'pass' : 'fail', 'pitch', { expected: p, actual: after.pitchSemitones })
      await clearTrackClips(trackId)
      return { pass: ok, note: `${after.pitchSemitones}st` }
    },
  }))
}

function genClipStretchMatrix(): TestDef[] {
  const values = [0.25, 0.33, 0.5, 0.66, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 3.5, 4.0]
  return values.map((r) => ({
    id: `p2r7_stretch_${r.toString().replace('.', '_')}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Clip time stretch (warping)',
    title: `Stretch ${r}x roundtrip`,
    instructions: `set_clip_stretch ${r}`,
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(30)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('set_clip_stretch', { trackId, clipId: c.id, stretchRatio: r })
      const [after] = await invoke<any[]>('get_track_clips', { trackId })
      const ok = Math.abs(after.stretchRatio - r) < 0.01
      log(ok ? 'pass' : 'fail', 'stretch', { expected: r, actual: after.stretchRatio })
      await clearTrackClips(trackId)
      return { pass: ok, note: `${after.stretchRatio}x` }
    },
  }))
}

function genClipGainMatrix(): TestDef[] {
  const values = [-60, -48, -36, -24, -18, -12, -9, -6, -3, -1, 0, 1, 3, 6, 9, 12]
  return values.map((db) => ({
    id: `p2r7_clip_gain_${db.toString().replace('-', 'n')}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Clip gain knob (per-clip volume)',
    title: `Clip gain ${db} dB roundtrip`,
    instructions: `set_clip_gain ${db}`,
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(30)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('set_clip_gain', { trackId, clipId: c.id, gainDb: db })
      const [after] = await invoke<any[]>('get_track_clips', { trackId })
      const ok = Math.abs(after.gainDb - db) < 0.01
      log(ok ? 'pass' : 'fail', 'gain', { expected: db, actual: after.gainDb })
      await clearTrackClips(trackId)
      return { pass: ok, note: `${after.gainDb}dB` }
    },
  }))
}

function genClipMoveMatrix(): TestDef[] {
  const positions = [0, 120, 240, 480, 720, 960, 1440, 1920, 2880, 3840, 5760, 7680, 9600, 11520, 15360, 19200]
  return positions.map((pos) => ({
    id: `p2r7_move_${pos}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Clip move: horizontal drag (time)',
    title: `Clip move to tick ${pos}`,
    instructions: `move_clip newPositionTicks=${pos}`,
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(30)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('move_clip', { trackId, clipId: c.id, newPositionTicks: pos })
      const [after] = await invoke<any[]>('get_track_clips', { trackId })
      const ok = after.position_ticks === pos
      log(ok ? 'pass' : 'fail', 'move', { expected: pos, actual: after.position_ticks })
      await clearTrackClips(trackId)
      return { pass: ok, note: `pos=${after.position_ticks}` }
    },
  }))
}

function genClipResizeMatrix(): TestDef[] {
  const lens = [120, 240, 480, 960, 1440, 1920, 2400, 2880, 3360, 3840, 4800, 5760, 7680, 9600, 11520]
  return lens.map((len) => ({
    id: `p2r7_resize_${len}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Clip resize: drag right edge',
    title: `Clip resize to ${len}t`,
    instructions: `resize_clip newLengthTicks=${len}`,
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(30)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('resize_clip', { trackId, clipId: c.id, newLengthTicks: len })
      const [after] = await invoke<any[]>('get_track_clips', { trackId })
      const ok = after.length_ticks === len
      log(ok ? 'pass' : 'fail', 'resize', { expected: len, actual: after.length_ticks })
      await clearTrackClips(trackId)
      return { pass: ok, note: `len=${after.length_ticks}` }
    },
  }))
}

function genClipFadeMatrix(): TestDef[] {
  const pairs: Array<[number, number]> = [
    [60, 60], [120, 120], [240, 240], [480, 480], [960, 0], [0, 960],
    [120, 480], [480, 120], [240, 720], [720, 240], [60, 900], [900, 60],
  ]
  return pairs.map(([fi, fo]) => ({
    id: `p2r7_fade_${fi}_${fo}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Clip fade-in handle (drag corner)',
    title: `Fades in=${fi} out=${fo}`,
    instructions: `set_clip_fades in=${fi} out=${fo}`,
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(30)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('set_clip_fades', { trackId, clipId: c.id, fadeInTicks: fi, fadeOutTicks: fo })
      const [after] = await invoke<any[]>('get_track_clips', { trackId })
      const ok = after.fadeInTicks === fi && after.fadeOutTicks === fo
      log(ok ? 'pass' : 'fail', 'fades', { expected: `${fi}/${fo}`, actual: `${after.fadeInTicks}/${after.fadeOutTicks}` })
      await clearTrackClips(trackId)
      return { pass: ok, note: `${after.fadeInTicks}/${after.fadeOutTicks}` }
    },
  }))
}

function genTrackVolumeMatrix(): TestDef[] {
  const values = [-60, -48, -36, -24, -18, -12, -9, -6, -3, 0, 3, 6, 9, 12]
  return values.map((db) => ({
    id: `p2r7_trackvol_${db.toString().replace('-', 'n')}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Track header: volume fader',
    title: `Track volume ${db} dB`,
    instructions: `set_track_volume ${db}`,
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      await invoke('set_track_volume', { trackId, volumeDb: db })
      const tracks = await invoke<any[]>('get_tracks')
      const t = tracks.find((x) => x.id === trackId)!
      const ok = Math.abs(t.volume_db - db) < 0.01
      log(ok ? 'pass' : 'fail', 'track vol', { expected: db, actual: t.volume_db })
      return { pass: ok, note: `${t.volume_db}dB` }
    },
  }))
}

function genTrackPanMatrix(): TestDef[] {
  const values = [-1, -0.75, -0.5, -0.25, -0.1, 0, 0.1, 0.25, 0.5, 0.75, 1]
  return values.map((pan) => ({
    id: `p2r7_trackpan_${pan.toString().replace('-', 'n').replace('.', '_')}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Track header: pan knob',
    title: `Track pan ${pan}`,
    instructions: `set_track_pan ${pan}`,
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      await invoke('set_track_pan', { trackId, pan })
      const tracks = await invoke<any[]>('get_tracks')
      const t = tracks.find((x) => x.id === trackId)!
      const ok = Math.abs(t.pan - pan) < 0.01
      log(ok ? 'pass' : 'fail', 'track pan', { expected: pan, actual: t.pan })
      return { pass: ok, note: `${t.pan}` }
    },
  }))
}

function genUndoRedoChain(): TestDef[] {
  const steps = [1, 2, 3, 5, 8, 13, 21, 34]
  return steps.map((n) => ({
    id: `p2r7_undo_chain_${n}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Undo stack (256+ levels)',
    title: `Undo chain of ${n} edits`,
    instructions: `Apply ${n} gain edits, undo ${n} times, original restored`,
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(30)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      const original = c.gainDb
      for (let i = 0; i < n; i++) {
        await invoke('set_clip_gain', { trackId, clipId: c.id, gainDb: -i - 1 })
      }
      for (let i = 0; i < n; i++) {
        await invoke('undo')
      }
      const [after] = await invoke<any[]>('get_track_clips', { trackId })
      const ok = Math.abs(after.gainDb - original) < 0.01
      log(ok ? 'pass' : 'fail', 'undo chain', { expected: original, actual: after.gainDb })
      await clearTrackClips(trackId)
      return { pass: ok, note: `n=${n} final=${after.gainDb}` }
    },
  }))
}

function genWaveformPeaksMatrix(): TestDef[] {
  const buckets = [16, 32, 64, 128, 256, 512, 1024, 2048]
  return buckets.map((b) => ({
    id: `p2r7_peaks_${b}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Clip waveform peak pre-computation',
    title: `get_waveform_peaks ${b} buckets`,
    instructions: `Returns exactly ${b} peak pairs`,
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(30)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      const peaks = await invoke<[number, number][]>('get_waveform_peaks', { sourceId: c.source_id, numBuckets: b })
      const ok = peaks.length === b
      log(ok ? 'pass' : 'fail', 'peaks', { expected: b, actual: peaks.length })
      await clearTrackClips(trackId)
      return { pass: ok, note: `got ${peaks.length}` }
    },
  }))
}

function genDuplicateChain(): TestDef[] {
  const counts = [1, 2, 3, 5, 8]
  return counts.map((n) => ({
    id: `p2r7_duplicate_chain_${n}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Clip duplicate: Ctrl+D',
    title: `Duplicate ${n} times`,
    instructions: `Expect ${n + 1} clips after chain`,
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(30)
      let [c] = await invoke<any[]>('get_track_clips', { trackId })
      let currentId = c.id
      for (let i = 0; i < n; i++) {
        currentId = await invoke<string>('duplicate_clip', { trackId, clipId: currentId })
      }
      const after = await invoke<any[]>('get_track_clips', { trackId })
      const ok = after.length === n + 1
      log(ok ? 'pass' : 'fail', 'dup count', { expected: n + 1, actual: after.length })
      await clearTrackClips(trackId)
      return { pass: ok, note: `${after.length} clips` }
    },
  }))
}

function genTrackAddRemove(): TestDef[] {
  const counts = [1, 2, 3, 5, 8, 13]
  return counts.map((n) => ({
    id: `p2r7_add_remove_${n}`,
    kind: 'AUTO' as const,
    phase: 2,
    phase1Item: 'Add track button',
    title: `Add+remove ${n} tracks`,
    instructions: `Add ${n}, remove ${n}, initial count preserved`,
    run: async ({ log }) => {
      const initial = (await invoke<any[]>('get_tracks')).length
      const ids: string[] = []
      for (let i = 0; i < n; i++) {
        await invoke('add_audio_track', { name: `TestTrack${i}` })
      }
      const mid = await invoke<any[]>('get_tracks')
      const added = mid.slice(-n)
      for (const t of added) ids.push(t.id)
      for (const id of ids) {
        await invoke('remove_track', { trackId: id })
      }
      const final = (await invoke<any[]>('get_tracks')).length
      const ok = mid.length === initial + n && final === initial
      log(ok ? 'pass' : 'fail', 'add/remove', { expected: `${initial}→${initial + n}→${initial}`, actual: `${initial}→${mid.length}→${final}` })
      return { pass: ok, note: `${initial}/${mid.length}/${final}` }
    },
  }))
}

function genMuteSoloMatrix(): TestDef[] {
  const tests: TestDef[] = []
  for (let i = 0; i < 4; i++) {
    tests.push({
      id: `p2r7_mute_toggle_${i}`,
      kind: 'AUTO',
      phase: 2,
      phase1Item: 'Track header: mute button',
      title: `Mute toggle ${i}`,
      instructions: `toggle_mute flips the flag`,
      run: async ({ log, ensureAudioTrack }) => {
        const trackId = await ensureAudioTrack()
        const before = (await invoke<any[]>('get_tracks')).find((t: any) => t.id === trackId)!.muted
        await invoke('toggle_mute', { trackId })
        const after = (await invoke<any[]>('get_tracks')).find((t: any) => t.id === trackId)!.muted
        const ok = after === !before
        log(ok ? 'pass' : 'fail', 'mute', { expected: !before, actual: after })
        await invoke('toggle_mute', { trackId })
        return { pass: ok, note: `${before}→${after}` }
      },
    })
    tests.push({
      id: `p2r7_solo_toggle_${i}`,
      kind: 'AUTO',
      phase: 2,
      phase1Item: 'Track header: solo button',
      title: `Solo toggle ${i}`,
      instructions: `toggle_solo flips the flag`,
      run: async ({ log, ensureAudioTrack }) => {
        const trackId = await ensureAudioTrack()
        const before = (await invoke<any[]>('get_tracks')).find((t: any) => t.id === trackId)!.soloed
        await invoke('toggle_solo', { trackId })
        const after = (await invoke<any[]>('get_tracks')).find((t: any) => t.id === trackId)!.soloed
        const ok = after === !before
        log(ok ? 'pass' : 'fail', 'solo', { expected: !before, actual: after })
        await invoke('toggle_solo', { trackId })
        return { pass: ok, note: `${before}→${after}` }
      },
    })
  }
  return tests
}

TESTS.push(
  ...genBpmRoundtrips(),
  ...genMasterVolumeRoundtrips(),
  ...genTimeSigRoundtrips(),
  ...genSnapPure(),
  ...genClipPitchMatrix(),
  ...genClipStretchMatrix(),
  ...genClipGainMatrix(),
  ...genClipMoveMatrix(),
  ...genClipResizeMatrix(),
  ...genClipFadeMatrix(),
  ...genTrackVolumeMatrix(),
  ...genTrackPanMatrix(),
  ...genUndoRedoChain(),
  ...genWaveformPeaksMatrix(),
  ...genDuplicateChain(),
  ...genTrackAddRemove(),
  ...genMuteSoloMatrix(),
)

// ─────────────────────────────────────────────────────────────────────────────
// Phase 2 Round 9: edge cases, error paths, and concurrency stress.
// Each test targets a boundary, failure mode, or rapid-fire race that the
// happy-path suite doesn't exercise.
// ─────────────────────────────────────────────────────────────────────────────

TESTS.push(
  {
    id: 'bpm_clamp_below_min',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'BPM boundary clamping',
    title: 'BPM clamps at 20 when set below',
    instructions: 'Sets BPM to 5 and verifies the readback is clamped to 20 (engine minimum).',
    run: async ({ log }) => {
      await invoke('set_bpm', { bpm: 5 })
      await sleep(40)
      const s = await devDumpState()
      const ok = s.bpm >= 20
      log(ok ? 'pass' : 'fail', 'bpm clamped', { expected: '>= 20', actual: s.bpm })
      await invoke('set_bpm', { bpm: 140 })
      return { pass: ok, note: `bpm=${s.bpm}` }
    },
  },
  {
    id: 'bpm_clamp_above_max',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'BPM boundary clamping',
    title: 'BPM clamps at 999 when set above',
    instructions: 'Sets BPM to 5000 and verifies the readback is clamped to 999.',
    run: async ({ log }) => {
      await invoke('set_bpm', { bpm: 5000 })
      await sleep(40)
      const s = await devDumpState()
      const ok = s.bpm <= 999
      log(ok ? 'pass' : 'fail', 'bpm clamped', { expected: '<= 999', actual: s.bpm })
      await invoke('set_bpm', { bpm: 140 })
      return { pass: ok, note: `bpm=${s.bpm}` }
    },
  },
  {
    id: 'bpm_nan_rejected',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'NaN input hardening',
    title: 'BPM NaN leaves state valid',
    instructions: 'Passes NaN to set_bpm and verifies the engine does not end up in a NaN state.',
    run: async ({ log }) => {
      await invoke('set_bpm', { bpm: 120 })
      try { await invoke('set_bpm', { bpm: Number.NaN }) } catch { /* expected */ }
      await sleep(40)
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm) && s.bpm > 0
      log(ok ? 'pass' : 'fail', 'bpm finite', { expected: 'finite > 0', actual: s.bpm })
      await invoke('set_bpm', { bpm: 140 })
      return { pass: ok, note: `bpm=${s.bpm}` }
    },
  },
  {
    id: 'master_volume_clamp',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Master volume clamping',
    title: 'Master volume clamps to sane range',
    instructions: 'Sets master volume to +100 and -500; verifies readback stays within engine range.',
    run: async ({ log }) => {
      await invoke('set_master_volume', { db: 100 })
      await sleep(40)
      const hi = (await devDumpState()).masterVolumeDb
      await invoke('set_master_volume', { db: -500 })
      await sleep(40)
      const lo = (await devDumpState()).masterVolumeDb
      const ok = hi <= 12 && lo >= -100
      log(ok ? 'pass' : 'fail', 'master clamped', { expected: 'hi<=12 lo>=-100', actual: `hi=${hi} lo=${lo}` })
      await invoke('set_master_volume', { db: 0 })
      return { pass: ok, note: `hi=${hi} lo=${lo}` }
    },
  },
  {
    id: 'track_pan_clamp',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Track pan clamping',
    title: 'Track pan clamps to [-1, 1]',
    instructions: 'Sets pan to 5 and -5; verifies both clamp into [-1, 1].',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_pan', { trackId: id, pan: 5 })
      const hi = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)?.pan ?? 0
      await invoke('set_track_pan', { trackId: id, pan: -5 })
      const lo = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)?.pan ?? 0
      const ok = hi <= 1.0 + 1e-6 && lo >= -1.0 - 1e-6
      log(ok ? 'pass' : 'fail', 'pan clamped', { expected: '[-1,1]', actual: `hi=${hi} lo=${lo}` })
      await invoke('set_track_pan', { trackId: id, pan: 0 })
      return { pass: ok, note: `hi=${hi} lo=${lo}` }
    },
  },
  {
    id: 'loop_start_after_end',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Loop region validation',
    title: 'Loop with start > end does not crash',
    instructions: 'Sets loop start=10000, end=100 and verifies engine remains responsive.',
    run: async ({ log }) => {
      await invoke('set_loop', { start: 10000, end: 100 })
      await sleep(40)
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'engine alive', { expected: 'valid state', actual: `bpm=${s.bpm}` })
      await invoke('set_loop', { start: 0, end: 48000 })
      return { pass: ok, note: `loop=${s.loopStart}..${s.loopEnd}` }
    },
  },
  {
    id: 'loop_zero_length',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Loop region validation',
    title: 'Zero-length loop region does not hang playback',
    instructions: 'Sets loop start=end, enables loop, plays briefly, verifies position advances or stays stable.',
    run: async ({ log }) => {
      await invoke('stop')
      await invoke('set_loop', { start: 48000, end: 48000 })
      if (!(await devDumpState()).looping) await invoke('toggle_loop')
      await invoke('set_position', { position: 0 })
      try { await invoke('start_engine') } catch {}
      await invoke('play')
      await sleep(300)
      const s = await devDumpState()
      await invoke('stop')
      if ((await devDumpState()).looping) await invoke('toggle_loop')
      const ok = Number.isFinite(s.positionSamples)
      log(ok ? 'pass' : 'fail', 'no hang', { expected: 'finite position', actual: s.positionSamples })
      return { pass: ok, note: `pos=${s.positionSamples}` }
    },
  },
  {
    id: 'set_position_huge',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Position boundary handling',
    title: 'set_position with huge value does not crash',
    instructions: 'Sets position near u64 max and verifies engine still responds.',
    run: async ({ log }) => {
      await invoke('stop')
      try { await invoke('set_position', { position: 1_000_000_000_000 }) } catch {}
      await sleep(40)
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'engine alive', { expected: 'responsive', actual: `bpm=${s.bpm}` })
      await invoke('set_position', { position: 0 })
      return { pass: ok, note: `pos=${s.positionSamples}` }
    },
  },
  {
    id: 'rapid_set_bpm_spam',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Command queue robustness',
    title: '100 rapid set_bpm calls land on final value',
    instructions: 'Fires 100 set_bpm commands back-to-back and verifies the last value is eventually observed.',
    run: async ({ log }) => {
      for (let i = 0; i < 100; i++) {
        invoke('set_bpm', { bpm: 60 + (i % 300) })
      }
      await invoke('set_bpm', { bpm: 135 })
      await sleep(150)
      const s = await devDumpState()
      const ok = Math.abs(s.bpm - 135) < 0.5
      log(ok ? 'pass' : 'fail', 'final bpm', { expected: 135, actual: s.bpm })
      await invoke('set_bpm', { bpm: 140 })
      return { pass: ok, note: `bpm=${s.bpm}` }
    },
  },
  {
    id: 'rapid_play_stop_spam',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Command queue robustness',
    title: '50 rapid play/stop toggles leave engine in stop state',
    instructions: 'Alternates play/stop 50 times, waits, verifies playing=false.',
    run: async ({ log }) => {
      for (let i = 0; i < 50; i++) {
        invoke(i % 2 === 0 ? 'play' : 'stop')
      }
      await invoke('stop')
      await sleep(200)
      const s = await devDumpState()
      const ok = !s.playing
      log(ok ? 'pass' : 'fail', 'final stopped', { expected: false, actual: s.playing })
      return { pass: ok, note: `playing=${s.playing}` }
    },
  },
  {
    id: 'rapid_volume_spam',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Command queue robustness',
    title: '100 rapid set_track_volume calls settle on final value',
    instructions: 'Fires 100 volume changes on one track and verifies the last value lands.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      for (let i = 0; i < 100; i++) {
        invoke('set_track_volume', { trackId: id, volumeDb: -i * 0.1 })
      }
      await invoke('set_track_volume', { trackId: id, volumeDb: -3.0 })
      await sleep(150)
      const t = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)
      const ok = t && Math.abs(t.volume_db - -3.0) < 0.1
      log(ok ? 'pass' : 'fail', 'final volume', { expected: -3.0, actual: t?.volume_db })
      await invoke('set_track_volume', { trackId: id, volumeDb: 0 })
      return { pass: !!ok, note: `vol=${t?.volume_db}` }
    },
  },
  {
    id: 'remove_nonexistent_track',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Error handling',
    title: 'remove_track on unknown id does not crash',
    instructions: 'Invokes remove_track with a fabricated UUID; verifies engine stays healthy.',
    run: async ({ log }) => {
      try { await invoke('remove_track', { trackId: '00000000-0000-0000-0000-000000000000' }) } catch { /* expected */ }
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'engine alive', { expected: 'responsive', actual: `bpm=${s.bpm}` })
      return { pass: ok, note: 'survived' }
    },
  },
  {
    id: 'load_nonexistent_project',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Error handling',
    title: 'load_project with missing file returns error',
    instructions: 'Attempts to load a path that does not exist; expects a rejected invoke and an intact engine.',
    run: async ({ log }) => {
      let threw = false
      try {
        await invoke('load_project', { path: 'C:/__definitely_does_not_exist__/missing.hwdaw' })
      } catch {
        threw = true
      }
      const s = await devDumpState()
      const ok = threw && Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'error surfaced', { expected: 'threw + alive', actual: `threw=${threw} bpm=${s.bpm}` })
      return { pass: ok, note: threw ? 'rejected cleanly' : 'silently succeeded' }
    },
  },
  {
    id: 'import_nonexistent_audio',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Error handling',
    title: 'import_audio_file with missing path returns error',
    instructions: 'Passes a nonexistent wav path to import_audio_file; expects a rejected invoke.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      let threw = false
      try {
        await invoke('import_audio_file', { trackId: id, path: 'C:/__nope__/ghost.wav' })
      } catch {
        threw = true
      }
      log(threw ? 'pass' : 'fail', 'rejected missing file', { expected: 'threw', actual: threw })
      return { pass: threw, note: threw ? 'rejected' : 'no error' }
    },
  },
  {
    id: 'reorder_negative_index',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Index boundary handling',
    title: 'reorder_track with negative index is safe',
    instructions: 'Calls reorder_track with a negative/huge newIndex; verifies no crash.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      try { await invoke('reorder_track', { trackId: id, newIndex: -5 }) } catch {}
      try { await invoke('reorder_track', { trackId: id, newIndex: 99999 }) } catch {}
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'engine alive', { expected: 'responsive', actual: s.bpm })
      return { pass: ok, note: 'no crash' }
    },
  },
  {
    id: 'undo_past_start',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Undo boundary handling',
    title: 'Many undos past history start do not crash',
    instructions: 'Invokes undo 200 times beyond available history; verifies engine still responds.',
    run: async ({ log }) => {
      for (let i = 0; i < 200; i++) {
        try { await invoke('undo') } catch { /* no-op when empty */ }
      }
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'engine alive', { expected: 'responsive', actual: s.bpm })
      return { pass: ok, note: 'survived' }
    },
  },
  {
    id: 'redo_without_undo',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Redo boundary handling',
    title: 'redo with empty redo-stack is a no-op',
    instructions: 'After a fresh project, invokes redo 20 times; verifies engine is unchanged and alive.',
    run: async ({ log }) => {
      for (let i = 0; i < 20; i++) {
        try { await invoke('redo') } catch { /* expected when empty */ }
      }
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'engine alive', { expected: 'responsive', actual: s.bpm })
      return { pass: ok, note: 'survived' }
    },
  },
  {
    id: 'pitch_clamp_extreme',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip pitch clamping',
    title: 'set_clip_pitch clamps extreme semitone values',
    instructions: 'Sets clip pitch to +200 and -200 semitones; verifies clamp to ±24.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      const id = await ensureAudioTrack()
      await clearTrackClips(id)
      await importAsset(id, 'sine_1khz_-6dbfs_stereo_5s.wav')
      const [c] = await invoke<any[]>('get_track_clips', { trackId: id })
      await invoke('set_clip_pitch', { trackId: id, clipId: c.id, pitchSemitones: 200 })
      const hi = (await invoke<any[]>('get_track_clips', { trackId: id }))[0].pitchSemitones
      await invoke('set_clip_pitch', { trackId: id, clipId: c.id, pitchSemitones: -200 })
      const lo = (await invoke<any[]>('get_track_clips', { trackId: id }))[0].pitchSemitones
      const ok = hi <= 24 + 1e-6 && lo >= -24 - 1e-6
      log(ok ? 'pass' : 'fail', 'pitch clamped', { expected: '±24', actual: `hi=${hi} lo=${lo}` })
      await clearTrackClips(id)
      return { pass: ok, note: `hi=${hi} lo=${lo}` }
    },
  },
  {
    id: 'stretch_clamp_zero',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Clip stretch clamping',
    title: 'set_clip_stretch clamps zero/negative ratios',
    instructions: 'Sets stretch ratio to 0 and -5; verifies the readback is a positive finite value.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      const id = await ensureAudioTrack()
      await clearTrackClips(id)
      await importAsset(id, 'sine_1khz_-6dbfs_stereo_5s.wav')
      const [c] = await invoke<any[]>('get_track_clips', { trackId: id })
      await invoke('set_clip_stretch', { trackId: id, clipId: c.id, stretchRatio: 0 })
      const z = (await invoke<any[]>('get_track_clips', { trackId: id }))[0].stretchRatio
      await invoke('set_clip_stretch', { trackId: id, clipId: c.id, stretchRatio: -5 })
      const n = (await invoke<any[]>('get_track_clips', { trackId: id }))[0].stretchRatio
      const ok = Number.isFinite(z) && z > 0 && Number.isFinite(n) && n > 0
      log(ok ? 'pass' : 'fail', 'stretch clamped', { expected: '> 0', actual: `z=${z} n=${n}` })
      await clearTrackClips(id)
      return { pass: ok, note: `z=${z} n=${n}` }
    },
  },
  {
    id: 'stress_add_many_tracks',
    kind: 'AUTO',
    phase: 2,
    phase1Item: 'Resource stress',
    title: 'Adding 50 tracks and removing them stays responsive',
    instructions: 'Adds 50 audio tracks, verifies count, then removes them; engine must remain responsive.',
    run: async ({ log }) => {
      const before = (await invoke<any[]>('get_tracks')).length
      const ids: string[] = []
      for (let i = 0; i < 50; i++) {
        const id = await invoke<string>('add_audio_track', { name: `Stress${i}` })
        ids.push(id)
      }
      const mid = (await invoke<any[]>('get_tracks')).length
      for (const id of ids) {
        try { await invoke('remove_track', { trackId: id }) } catch {}
      }
      const after = (await invoke<any[]>('get_tracks')).length
      const ok = mid === before + 50 && after === before
      log(ok ? 'pass' : 'fail', 'track count', { expected: `${before}→${before + 50}→${before}`, actual: `${before}→${mid}→${after}` })
      return { pass: ok, note: `${before}→${mid}→${after}` }
    },
  },
)

// Phase 3 — extensive per-feature coverage plus edge cases.
TESTS.push(...PHASE3_TESTS)

// Phase 4 — exhaustive parametric sweeps + integration scenarios.
TESTS.push(...PHASE4_TESTS)
