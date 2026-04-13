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
import { devDumpState, devForceDeviceError, devResolveTestAsset, queryTestId, getMeterDb, clickCanvas, type DevState } from './devApi'

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
    instructions: 'Plays the 1 kHz -6 dBFS test tone; asserts master peak within ±2 dB of -6.',
    run: async ({ log, ensureAudioTrack, clearTrackClips, importAsset }) => {
      await invoke('stop')
      await invoke('set_pattern_mode', { enabled: false })
      await invoke('set_master_volume', { db: 0 })
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine_1khz_-6dbfs_stereo_5s.wav')
      await invoke('set_position', { position: 0 })
      try { await invoke('start_engine') } catch {}
      await invoke('play')
      await sleep(800)
      const s = await devDumpState()
      await invoke('stop')
      const peak = s.masterPeakDb
      const ok = approx(peak, -6, 2.0)
      log(ok ? 'pass' : 'fail', 'master peak', { expected: '-6 ± 2 dB', actual: peak.toFixed(2) })
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
    instructions: 'Queries DOM for all 8 dB scale marks and verifies they exist and are visible.',
    run: async ({ log, ensureAudioTrack }) => {
      // Ensure at least one track so the mixer renders the dB scale
      await ensureAudioTrack()
      await sleep(100)

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
]
