/**
 * Dev-only Tauri IPC mock for the screenshot harness (screenshot.html).
 *
 * `@tauri-apps/api/core` `invoke()` reads `window.__TAURI_INTERNALS__.invoke`
 * lazily at call time, so installing this on import — before the harness
 * mounts and effects fire — is enough to satisfy the handful of commands the
 * Toolbar + Arrangement poll. NOT part of the shipped app: only screenshot.tsx
 * imports it, and only screenshot.html loads that entry (dev server only).
 */

// Synthetic waveform peaks: [min, max, rms, brightness]. Models a realistic
// drum-loop-ish signal — several transient hits, each a bright onset (treble
// → blue) decaying into a bass body (red), with per-sample jitter so the
// waveform has the jagged detail of real audio rather than a smooth blob.
function synthPeaks(n: number): [number, number, number, number][] {
  // Deterministic pseudo-random so screenshots are reproducible.
  const rand = (i: number) => {
    const x = Math.sin(i * 127.1 + 311.7) * 43758.5453
    return x - Math.floor(x)
  }
  const hits = [0.0, 0.18, 0.31, 0.5, 0.63, 0.75, 0.88]
  const out: [number, number, number, number][] = []
  for (let i = 0; i < n; i++) {
    const t = i / Math.max(1, n)
    // Nearest preceding hit → transient envelope + brightness.
    let env = 0
    let bright = 0.08
    for (const hp of hits) {
      const d = t - hp
      if (d >= 0) {
        const e = Math.exp(-d * 26)
        if (e > env) {
          env = e
          bright = Math.min(1, e * 0.95 + 0.08) // bright at onset, decays to bass
        }
      }
    }
    const jitter = 0.45 + 0.55 * rand(i) // per-bucket detail
    const amp = Math.min(1, env * jitter)
    const max = amp
    const min = -amp * (0.82 + 0.18 * rand(i + 7))
    const rms = amp * (0.5 + 0.35 * rand(i + 13))
    out.push([min, max, rms, bright])
  }
  return out
}

interface TauriInternals {
  transformCallback: (cb: unknown) => unknown
  unregisterCallback: (id: unknown) => void
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>
}

/** `@tauri-apps/api/event` bookkeeping the real webview injects. */
interface TauriEventPluginInternals {
  unregisterListener: (event: string, eventId: unknown) => void
}

// A dense 8-bar lead so the piano roll reads like a real session (arp line
// spanning ~3 octaves + sustained chord stabs) — matters for marketing shots.
function synthNotes() {
  const PPQ = 960
  const notes: Array<Record<string, number | boolean>> = []
  const scale = [48, 51, 55, 58, 60, 63, 67, 70, 72, 75, 79, 82, 84] // C minor-ish
  let idx = 0
  // 16th-note arp over 8 bars, rising/falling
  for (let step = 0; step < 128; step++) {
    const wave = Math.round((scale.length - 1) * Math.abs(Math.sin(step / 9)))
    notes.push({
      index: idx++,
      start_tick: Math.floor((step * PPQ) / 4),
      duration_ticks: Math.floor(PPQ / 4) - 30,
      pitch: scale[wave],
      velocity: (74 + ((step * 13) % 48)) / 127,
      channel: 0,
      muted: false,
      // Every other hit steps to the side, which is what the pan strip
      // is for and what the screenshot has to show.
      pan: step % 2 === 0 ? -0.6 : 0.6,
      fine_cents: step % 8 === 0 ? 4 : 0,
      release_velocity: 0.5,
    })
  }
  // chord stabs every bar (triads, held half a bar)
  for (let bar = 0; bar < 8; bar++) {
    for (const p of [36, 43, 48]) {
      notes.push({
        index: idx++,
        start_tick: bar * PPQ * 4,
        duration_ticks: PPQ * 2,
        pitch: p + (bar % 2 === 0 ? 0 : 3),
        velocity: 96 / 127,
        channel: 0,
        muted: false,
      })
    }
  }
  return notes
}

// Browser Places tree: a small fake sample library for the screenshot
// harness and Playwright. Shape matches the Rust `BrowserEntry`.
function mockDirectory(path: string) {
  const entry = (name: string, isDir: boolean, sizeBytes = 0) =>
    ({ name, path: `${path}/${name}`, isDir, sizeBytes })
  const leaf = path.split('/').pop() ?? ''
  if (leaf === 'Kicks') {
    return ['Kick Hard 01.wav', 'Kick Hard 02.wav', 'Kick Rawstyle 03.wav', 'Kick Tail 04.wav']
      .map(n => entry(n, false, 412_000))
  }
  if (leaf === 'Snares') return ['Clap 01.wav', 'Snare Roll 140.wav'].map(n => entry(n, false, 236_000))
  if (leaf === 'Samples') {
    return [entry('Kicks', true), entry('Loops', true), entry('Snares', true), entry('Screech 150 F.wav', false, 1_840_000)]
  }
  return []
}

const mock: TauriInternals = {
  transformCallback: (cb) => cb,
  unregisterCallback: () => {},
  invoke: async (cmd, args) => {
    switch (cmd) {
      case 'get_file_peaks': {
        // A decaying shape, so the browser's thumbnails can be photographed.
        return Array.from({ length: 48 }, (_, i) => {
          const a = Math.exp(-i / 14) * (0.4 + 0.6 * Math.abs(Math.sin(i * 1.7)))
          return [-a, a]
        })
      }
      case 'get_waveform_peaks':
        return synthPeaks((args?.numBuckets as number) ?? 256)
      case 'get_clip_controls': {
        // A mod-wheel swell so the controller lane can be photographed.
        const a = args as { kind?: string }
        if (a?.kind !== 'cc') return []
        return Array.from({ length: 33 }, (_, i) => ({
          tick: i * 60,
          value: Math.sin((i / 32) * Math.PI) * 0.9,
        }))
      }
      case 'set_clip_controls':
        return null
      case 'get_midi_notes':
        return synthNotes()
      case 'search_library': {
        const q = String((args as { query?: string })?.query ?? '').toLowerCase()
        const pool = ['Kick Hard 01.wav', 'Kick Rawstyle 03.wav', 'Screech 150 F.wav']
        return {
          matches: pool.filter(n => n.toLowerCase().includes(q)).map(n => ({
            name: n, path: `/samples/${n}`, isDir: false, sizeBytes: 1024,
          })),
          hitLimit: false,
        }
      }
      case 'comp_take_range':
        return 3
      case 'spread_takes_to_lanes':
        return 2
      case 'list_vcas':
        return [
          { id: 'v1', name: 'Drums', gain_db: -3.5, muted: false, members: ['insert-001', 'insert-002'] },
          { id: 'v2', name: 'Leads', gain_db: 0, muted: false, members: ['insert-003'] },
        ]
      case 'add_vca':
        return 'v3'
      case 'set_vca_gain':
      case 'set_vca_muted':
      case 'set_vca_members':
      case 'rename_vca':
      case 'delete_vca':
        return null
      case 'get_mpe':
        return false
      case 'set_mpe':
        return null
      case 'get_tuning':
        return null
      case 'load_tuning_file':
        return { name: 'Phrygian 12', degrees_cents: [100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 1100, 1200], root_note: 60, root_hz: 261.6 }
      case 'clear_tuning':
      case 'set_tuning_root':
        return null
      case 'list_modulations':
        return [
          {
            id: 'm1', name: 'Cutoff 1/4', enabled: true,
            source: { Lfo: { shape: 'Sine', rate: { TempoSync: { num: 1, den: 4 } }, phase_offset: 0 } },
            track_id: 'insert-001',
            target: { PluginParam: { slot_id: 'sl1', param_id: 4 } },
            center: 0.5, depth: 0.35,
          },
        ]
      case 'add_modulation':
        return 'm2'
      case 'set_modulation':
      case 'delete_modulation':
        return null
      case 'align_track_to':
        return { offsetSamples: 73, offsetMs: 1.52, correlation: 0.97, polarityFlipped: false }
      case 'get_reference':
        return { loaded: true, playing: false, gainDb: -2.4, lufs: -8.1, name: 'Rawstyle reference' }
      case 'load_reference':
        return { loaded: true, playing: false, gainDb: 0, lufs: -8.1, name: 'Rawstyle reference' }
      case 'set_reference_playing':
      case 'set_reference_gain':
      case 'clear_reference':
        return null
      case 'match_reference_loudness':
        return -2.4
      case 'get_link_status':
        return { enabled: false, peers: 0 }
      case 'set_link_enabled':
        return null
      case 'get_worker_threads':
        return { threads: 0, suggested: 5, cores: 8 }
      case 'set_worker_threads':
        return null
      case 'get_control_surface':
        return { enabled: false, bank: 0, bankCount: 3 }
      case 'set_control_surface_enabled':
      case 'set_control_surface_bank':
        return null
      case 'get_channel_offsets':
        return { input: 0, output: 0 }
      case 'set_channel_offsets':
        return null
      case 'drain_plugin_knob_moves':
        return []
      case 'list_frozen_tracks':
        return ['t-bass']
      case 'freeze_track':
        return '/freeze/bass.wav'
      case 'unfreeze_track':
        return null
      case 'get_midi_fx':
        return [
          { Arpeggiator: { step_ticks: 240, mode: 'Up', gate: 0.9, octaves: 2 } },
          { Scale: { root: 4, kind: 'PhrygianDominant' } },
        ]
      case 'set_midi_fx':
        return null
      case 'preview_midi_fx':
        return []
      case 'get_midi_routes':
        return []
      case 'set_midi_routes':
        return null
      case 'list_factory_presets':
        return ['Init', 'Hard Screech', 'Sub Bass', 'Reverse Bell']
      case 'load_factory_preset':
        return null
      case 'list_all_presets':
        return [
          {
            pluginId: 'hardwave.wettboi',
            presets: [
              { id: 'p1', name: 'Rawstyle screech', created_at: 1758900000 },
              { id: 'p2', name: 'Wide pad', created_at: 1758800000 },
            ],
          },
          {
            pluginId: 'hardwave.loudlab',
            presets: [{ id: 'p3', name: 'Club master', created_at: 1758700000 }],
          },
        ]
      case 'list_macros':
        return [
          {
            id: 'm1', name: 'Open', value: 0.42,
            links: [
              { id: 'l1', track_id: 'insert-001', target: { PluginParam: { slot_id: 'sl1', param_id: 4 } }, min: 200, max: 8000 },
              { id: 'l2', track_id: 'insert-002', target: 'TrackVolume', min: -18, max: -3 },
            ],
          },
          { id: 'm2', name: 'Wide', value: 0, links: [] },
        ]
      case 'get_plugin_parameters':
        return [
          { id: 0, name: 'Drive', defaultValue: 0.3, value: 0.3, min: 0, max: 1, unit: '', automatable: true },
          { id: 4, name: 'Cutoff', defaultValue: 1200, value: 1200, min: 20, max: 20000, unit: 'Hz', automatable: true },
        ]
      case 'add_macro':
      case 'add_macro_link':
        return 'new-id'
      case 'set_macro_value':
      case 'set_macro_link_range':
      case 'remove_macro_link':
      case 'rename_macro':
      case 'delete_macro':
      case 'apply_all_macros':
        return null
      case 'list_sections':
        return [
          { id: 's1', name: 'Intro', startTicks: 0, endTicks: 7680 },
          { id: 's2', name: 'Drop', startTicks: 7680, endTicks: 15360 },
        ]
      case 'list_directory':
        return mockDirectory(String(args?.path ?? ''))
      case 'plugin:dialog|open':
        return '/Users/producer/Samples'
      // List-shaped commands must return arrays, not null (consumers iterate).
      case 'list_sends':
      case 'get_sends':
      case 'list_arrangements':
        return []
      // Boot path for the FULL app under Playwright (main.tsx loads this
      // mock in browser/dev mode since 2026-07-07): every command the
      // splash-to-idle sequence awaits must resolve with the right SHAPE
      // or the app never leaves the splash and all UI specs fail.
      case 'get_track_with_clips':
        return null
      case 'get_tracks':
      case 'get_tracks_with_clips':
      case 'find_missing_plugins':
      case 'scan_plugins':
      case 'get_plugins':
        return []
      case 'get_transport_state':
        return {
          playing: false, recording: false, looping: false,
          position_samples: 0, bpm: 140,
          // The screenshot harness can ask for another signature; without
          // this the poll would overwrite whatever it seeded a moment later,
          // and a grid change could never be photographed.
          time_sig_numerator: (window as unknown as { __HW_TIMESIG__?: number }).__HW_TIMESIG__ ?? 4,
          time_sig_denominator: 4, master_volume_db: 0, pattern_mode: false,
        }
      case 'bug_report_env':
        return { version: '0.0.0-dev', os: 'Linux', logAvailable: false }
      case 'session_log_tail':
        return ''
      // Nothing is missing in a mocked project.
      case 'list_missing_sources':
        return []
      case 'auto_relink_sources':
        return []
      case 'relink_source':
        return 0
      case 'collect_project_samples':
        return { copied: 0, alreadyThere: 0, missing: [], folder: 'Project Samples' }
      // Markers + punch range now ride in the project, not localStorage.
      case 'get_timeline_state':
        return null
      case 'set_timeline_state':
        return null
      case 'get_project_meta':
        return { show_on_open: false }
      case 'get_custom_scan_paths':
        return [[], []]
      case 'diagnostics_info':
        return { logsDir: '/tmp/mock-logs', currentSessionLog: null }
      case 'midi_driver_status':
        return { ok: true, detail: '' }
      case 'set_pan_law':
        return null
      case 'set_automation_trim':
      case 'get_automation_trim':
        return false
      case 'set_reset_on_transport':
      case 'set_play_truncated_notes':
        return null
      case 'set_midi_master_enabled':
      case 'set_midi_velocity_curve':
        return null
      case 'get_midi_master_enabled':
        return true
      case 'get_midi_velocity_curve':
        return 'linear'
      case 'get_tempo_entries':
        // The screenshot harness seeds this so a mid-song signature change
        // can be photographed; without it the playlist refreshes the map at
        // mount and paints 4/4 over whatever was seeded.
        return (window as unknown as { __HW_TEMPO_ENTRIES__?: unknown[] }).__HW_TEMPO_ENTRIES__
          ?? [{ tick: 0, bpm: 140, timeSigNum: 4, timeSigDen: 4, ramp: 'instant' }]
      case 'set_tempo_entry_time_signature':
        return null
      case 'preview_audio_file_in_tempo':
        return { fileBpm: 150, speed: 0.93 }
      case 'preview_audio_file':
      case 'stop_audio_preview':
      case 'set_preview_volume':
        return null
      case 'set_clip_muted':
        return true
      case 'slip_clip':
        return 0
      case 'get_count_in_state':
        return { active: false, beat: 0, totalBeats: 0 }
      case 'start_count_in':
        return null
      case 'set_metronome_enabled':
      case 'set_metronome_volume':
      case 'set_metronome_accent':
      case 'set_metronome_record_only':
        return null
      case 'set_automation_write_mode':
      case 'automation_touch_begin':
      case 'automation_write_sample':
        return null
      case 'get_automation_write_mode':
        return 'off'
      case 'automation_touch_end':
        return null
      case 'set_punch_range':
        return null
      case 'get_punch_range':
        return [false, 0, 0]
      case 'begin_history_group':
      case 'end_history_group':
        return null
      case 'process_memory':
        return { usedBytes: 412 * 1024 * 1024, totalBytes: 16 * 1024 * 1024 * 1024 }
      case 'get_audio_load':
        return { loadPct: 12, xruns: 0 }
      case 'get_graph_latency':
        return { samples: 0, ms: 0, pdcEnabled: true }
      // Setup-wizard audio step (screenshot harness renders it headless).
      case 'list_audio_hosts':
        return ['WASAPI', 'ASIO']
      case 'get_audio_host':
        return 'WASAPI'
      case 'set_audio_host':
        return null
      case 'get_audio_devices':
        return [
          { name: 'Focusrite Scarlett 2i2', is_default: false, sample_rates: [44100, 48000, 96000], max_channels: 2 },
          { name: 'Speakers (Realtek HD Audio)', is_default: true, sample_rates: [44100, 48000], max_channels: 2 },
        ]
      case 'get_audio_config':
        return { device: 'Focusrite Scarlett 2i2', sample_rate: 48000, buffer_size: 512 }
      // Settings window (screenshot harness renders it headless).
      case 'get_audio_input_devices':
        return [{ name: 'Focusrite Scarlett 2i2', is_default: true, sample_rates: [44100, 48000, 96000], max_channels: 2 }]
      case 'get_audio_input_config':
        return { device: 'Focusrite Scarlett 2i2', channels: 2 }
      case 'get_wasapi_exclusive':
        return { enabled: false, available: true }
      case 'list_midi_outputs':
        return ['Microsoft GS Wavetable Synth']
      case 'get_midi_clock_status':
        return { enabled: false, open_ports: [] }
      case 'get_midi_clock_sync_status':
        return { enabled: false, ticks_seen: false, last_bpm: null }
      case 'get_midi_mtc_status':
        return { enabled: false, fps: 25 }
      case 'get_direct_monitoring':
        return false
      case 'get_audio_cache_stats':
        return { bytesUsed: 96 * 1024 * 1024, maxBytes: 512 * 1024 * 1024, entryCount: 14 }
      case 'get_recording_latency':
        return { outputMs: 10.7, inputMs: 10.7, queueMs: 2.1, offsetMs: 0, totalMs: 23.5, inputRunning: true }
      case 'get_input_meter':
        return { peak_l: 0, peak_r: 0, running: false, sample_rate: 48000, buffer_size: 512 }
      case 'list_midi_inputs':
        // Two named ports so the setup wizard's device and velocity steps can
        // be photographed with something in them.
        return ['Akai MPK Mini mk3', 'Novation Launchkey 49']
      case 'get_midi_activity':
        return { open_ports: [], ms_since_last_event: null }
      // Event plugin — let listeners register harmlessly.
      case 'plugin:event|listen':
        return 0
      case 'plugin:event|unlisten':
        return null
      default:
        return null
    }
  },
}

// Install ONLY when no real backend exists. Inside actual Tauri the
// webview injects __TAURI_INTERNALS__ before any module runs, so this
// import is a safe no-op there — which is what lets main.tsx import
// the mock unconditionally for browser/Playwright boots.
{
  const w = window as unknown as {
    __TAURI_INTERNALS__?: TauriInternals
    __TAURI_EVENT_PLUGIN_INTERNALS__?: TauriEventPluginInternals
  }
  if (!w.__TAURI_INTERNALS__) {
    w.__TAURI_INTERNALS__ = mock
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }
  }
}
