use parking_lot::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::{Emitter, Manager};

mod commands;
mod frontend_updater;
mod midi_clock;
mod midi_map;
mod midi_sync;
mod midi_timecode;
mod prefs;

use hardwave_engine::DawEngine;
pub use midi_clock::MidiClockState;
pub use midi_map::MidiMappings;
pub use midi_sync::MidiClockSyncState;
pub use midi_timecode::MidiTimecodeState;
pub use prefs::AudioPrefs;

/// Shared engine state accessible from Tauri commands.
pub struct AppState {
    pub engine: Arc<Mutex<DawEngine>>,
    /// Flipped by `cancel_export` to halt an in-progress offline render.
    /// The export command clears it on entry and checks it each block.
    pub export_cancel: Arc<AtomicBool>,
    pub midi_mappings: Arc<Mutex<MidiMappings>>,
    pub midi_clock: Arc<MidiClockState>,
    pub midi_sync: Arc<MidiClockSyncState>,
    pub midi_timecode: Arc<MidiTimecodeState>,
    /// Live plugin editor instances, keyed by the Tauri window label
    /// they're parented to. Dropping an entry closes the editor view.
    pub plugin_editors: Arc<
        Mutex<
            std::collections::HashMap<String, Box<dyn hardwave_plugin_host::types::HostedPlugin>>,
        >,
    >,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    let mut engine = DawEngine::new();
    // Apply persisted audio preferences before the engine starts so the first
    // stream honors the user's last device choice instead of the default.
    let prefs = AudioPrefs::load();
    if let Err(e) = engine.set_audio_config(
        prefs.output_device.clone(),
        prefs.sample_rate,
        prefs.buffer_size,
    ) {
        log::warn!("Failed to apply saved audio output prefs: {e}");
    }
    engine.set_input_config(prefs.input_device.clone(), prefs.input_channels);
    #[cfg(target_os = "windows")]
    if prefs.wasapi_exclusive {
        if let Err(e) = engine.set_wasapi_exclusive(true) {
            log::warn!("Failed to apply saved WASAPI exclusive pref: {e}");
        }
    }
    let midi_mappings = Arc::new(Mutex::new(MidiMappings::load()));
    let midi_clock = Arc::new(MidiClockState::new());
    let midi_sync = Arc::new(MidiClockSyncState::new());
    let midi_timecode = Arc::new(MidiTimecodeState::with_output(Arc::clone(
        &midi_clock.output,
    )));
    let state = AppState {
        engine: Arc::new(Mutex::new(engine)),
        export_cancel: Arc::new(AtomicBool::new(false)),
        plugin_editors: Arc::new(Mutex::new(std::collections::HashMap::new())),
        midi_mappings: Arc::clone(&midi_mappings),
        midi_clock: Arc::clone(&midi_clock),
        midi_sync: Arc::clone(&midi_sync),
        midi_timecode: Arc::clone(&midi_timecode),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        // Custom protocol that serves the frontend bundle from the local
        // cache when the updater has staged a newer version, falling back
        // to the bundled assets when there's no cache yet. Default
        // tauri://localhost still works so this is purely additive.
        .register_uri_scheme_protocol(frontend_updater::PROTOCOL_SCHEME, |ctx, request| {
            frontend_updater::handle_request(ctx.app_handle(), &request)
        })
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            // Transport
            commands::transport::play,
            commands::transport::stop,
            commands::transport::set_position,
            commands::transport::set_bpm,
            commands::transport::toggle_loop,
            commands::transport::set_loop,
            commands::transport::set_master_volume,
            commands::transport::set_time_signature,
            commands::transport::set_pattern_mode,
            commands::transport::get_transport_state,
            // Tracks
            commands::tracks::get_tracks,
            commands::tracks::add_audio_track,
            commands::tracks::add_midi_track,
            commands::tracks::add_automation_track,
            commands::tracks::remove_track,
            commands::tracks::set_track_volume,
            commands::tracks::set_track_pan,
            commands::tracks::toggle_mute,
            commands::tracks::toggle_solo,
            commands::tracks::set_exclusive_solo,
            commands::tracks::toggle_solo_safe,
            commands::tracks::toggle_arm,
            commands::tracks::set_track_monitor_input,
            commands::tracks::reorder_track,
            commands::tracks::set_track_name,
            commands::tracks::set_track_color,
            commands::tracks::set_track_phase_invert,
            commands::tracks::set_track_swap_lr,
            commands::tracks::set_track_stereo_separation,
            commands::tracks::set_track_delay_samples,
            commands::tracks::set_track_pitch_semitones,
            commands::tracks::set_track_fine_tune_cents,
            commands::tracks::set_track_filter_type,
            commands::tracks::set_track_filter_cutoff,
            commands::tracks::set_track_filter_resonance,
            commands::tracks::set_track_output_bus,
            // Sends
            commands::sends::get_sends,
            commands::sends::list_sends,
            commands::sends::add_send,
            commands::sends::remove_send,
            commands::sends::set_send_target,
            commands::sends::set_send_gain,
            commands::sends::set_send_pre_fader,
            commands::sends::set_send_enabled,
            commands::sends::create_return_with_send,
            // Plugins
            commands::plugins::scan_plugins,
            commands::plugins::get_plugins,
            commands::plugins::get_last_scan_diff,
            commands::plugins::get_plugin_blocklist,
            commands::plugins::set_plugin_blocklist,
            commands::plugins::get_custom_scan_paths,
            commands::plugins::set_custom_scan_paths,
            commands::plugins::plugin_cache_path,
            commands::plugins::add_plugin_to_track,
            commands::plugins::remove_plugin_from_track,
            commands::plugins::open_plugin_editor,
            commands::plugins::close_plugin_editor,
            commands::plugins::set_insert_enabled,
            commands::plugins::reorder_insert,
            commands::plugins::set_fx_chain_bypassed,
            commands::plugins::set_insert_wet,
            commands::plugins::set_plugin_sidechain_source,
            commands::plugins::find_missing_plugins,
            commands::engine::get_graph_latency,
            commands::engine::get_pdc_enabled,
            commands::engine::set_pdc_enabled,
            commands::engine::get_audio_cache_stats,
            commands::engine::set_audio_cache_max_bytes,
            // Project
            commands::project::new_project,
            commands::project::save_project,
            commands::project::load_project,
            commands::project::get_project_info,
            commands::project::get_channel_rack_state,
            commands::project::set_channel_rack_state,
            commands::project::get_tempo_entries,
            commands::project::add_tempo_entry,
            commands::project::remove_tempo_entry,
            commands::project::set_tempo_entry,
            // Autosave / crash recovery
            commands::autosave::autosave_save,
            commands::autosave::autosave_latest,
            commands::autosave::autosave_clear,
            commands::autosave::autosave_mark_alive,
            commands::autosave::autosave_clear_alive,
            commands::autosave::autosave_detect_crash,
            // Engine
            commands::engine::start_engine,
            commands::engine::stop_engine,
            commands::engine::get_meters,
            commands::engine::get_master_samples,
            commands::engine::get_audio_devices,
            commands::engine::get_audio_config,
            commands::engine::set_audio_config,
            commands::engine::get_audio_input_devices,
            commands::engine::get_audio_input_config,
            commands::engine::set_audio_input_config,
            commands::engine::start_input_monitoring,
            commands::engine::stop_input_monitoring,
            commands::engine::set_direct_monitoring,
            commands::engine::get_direct_monitoring,
            commands::engine::get_input_meter,
            commands::engine::list_audio_hosts,
            commands::engine::get_audio_host,
            commands::engine::set_audio_host,
            commands::engine::get_wasapi_exclusive,
            commands::engine::set_wasapi_exclusive,
            // Audio
            commands::audio::import_audio_file,
            commands::audio::get_track_clips,
            commands::audio::get_waveform_peaks,
            commands::audio::move_clip,
            commands::audio::resize_clip,
            commands::audio::delete_clip,
            commands::audio::duplicate_clip,
            commands::audio::split_clip,
            commands::audio::set_clip_gain,
            commands::audio::set_clip_fades,
            commands::audio::set_clip_fade_curves,
            commands::audio::toggle_clip_reverse,
            commands::audio::set_clip_pitch,
            commands::audio::set_clip_stretch,
            // MIDI
            commands::midi::create_midi_clip,
            commands::midi::get_midi_notes,
            commands::midi::add_midi_note,
            commands::midi::update_midi_note,
            commands::midi::delete_midi_note,
            // MIDI input (live)
            commands::midi_input::list_midi_inputs,
            commands::midi_input::open_midi_input,
            commands::midi_input::close_midi_input,
            commands::midi_input::close_all_midi_inputs,
            commands::midi_input::get_midi_activity,
            commands::midi_input::get_midi_desired_ports,
            commands::midi_input::set_midi_clock_sync_enabled,
            commands::midi_input::get_midi_clock_sync_status,
            // MIDI Learn
            commands::midi_learn::midi_learn_start,
            commands::midi_learn::midi_learn_cancel,
            commands::midi_learn::midi_learn_status,
            commands::midi_learn::list_midi_mappings,
            commands::midi_learn::remove_midi_mapping,
            commands::midi_learn::clear_midi_mappings,
            // MIDI Clock output
            commands::midi_output::list_midi_outputs,
            commands::midi_output::open_midi_output,
            commands::midi_output::close_midi_output,
            commands::midi_output::set_midi_clock_enabled,
            commands::midi_output::get_midi_clock_status,
            commands::midi_output::set_midi_mtc_enabled,
            commands::midi_output::set_midi_mtc_fps,
            commands::midi_output::get_midi_mtc_status,
            // Undo/redo
            commands::history::undo,
            commands::history::redo,
            commands::history::history_sizes,
            // Export
            commands::export::export_project_wav,
            commands::export::cancel_export,
            commands::export::export_project_stems,
            // Dev panel (stripped before merge to master)
            commands::dev::dev_dump_state,
            commands::dev::dev_force_device_error,
            commands::dev::dev_resolve_test_asset,
            commands::dev::dev_list_test_assets,
            // Frontend updater — splash-driven hot-swap of the UI bundle
            frontend_updater::frontend_update_check_and_apply,
            frontend_updater::frontend_update_status,
            // Version contract resolver — single-source-of-truth decision
            // between Path A (installer modal) and Path B (hot-swap).
            frontend_updater::version_contract_state,
        ])
        .setup(|app| {
            log::info!("Hardwave DAW starting");

            // Frontend hot-swap: if the updater has previously staged a
            // newer bundle (active.txt + <version>/index.html present),
            // navigate the main window to the custom protocol so the
            // user sees the cached UI on this launch. No-op when the
            // cache is empty — the bundled UI keeps loading.
            frontend_updater::maybe_activate_cache(&app.handle().clone());

            // Start meter broadcast thread
            let state = app.state::<AppState>();
            let engine = Arc::clone(&state.engine);
            let app_handle = app.handle().clone();

            // MIDI Learn dispatcher: drains incoming CC events and applies
            // mapped values to the live engine state. Also handles learn-mode
            // capture in the same loop so there's no race with the main
            // meter/transport broadcast thread.
            midi_map::spawn_dispatcher(Arc::clone(&state.engine), Arc::clone(&state.midi_mappings));

            // MIDI Clock dispatcher: sends 24 PPQN clock ticks and
            // Start/Stop system realtime messages to every open MIDI output
            // whenever the user has enabled clock send in Audio settings.
            midi_clock::spawn_dispatcher(Arc::clone(&state.engine), Arc::clone(&state.midi_clock));

            // MIDI Clock sync dispatcher: observes clock ticks from the
            // MIDI input manager and, when sync is enabled, slaves the
            // transport BPM and play/stop state to the external master.
            midi_sync::spawn_dispatcher(Arc::clone(&state.engine), Arc::clone(&state.midi_sync));

            // MIDI Time Code dispatcher: emits Quarter Frame messages at 4×fps
            // while playing and MTC send is enabled. Uses the same output
            // manager as MidiClockState so both stream to the same ports.
            midi_timecode::spawn_dispatcher(
                Arc::clone(&state.engine),
                Arc::clone(&state.midi_timecode),
            );

            // MIDI hot-plug reconciler: every ~2s, reopens desired ports that
            // have come back online and drops connections whose device has
            // vanished. Desired set is maintained by open/close commands so
            // the user's choice survives unplug/replug.
            {
                let engine_for_reconcile = Arc::clone(&state.engine);
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_millis(2000));
                    let mgr = {
                        let eng = engine_for_reconcile.lock();
                        Arc::clone(&eng.midi_input)
                    };
                    let _report = mgr.lock().reconcile();
                });
            }

            // Load the plugin cache from disk, then kick off a background
            // rescan so added/removed plugins are detected on startup without
            // blocking the UI. Persists the fresh cache after the scan.
            {
                let engine_for_scan = Arc::clone(&state.engine);
                std::thread::spawn(move || {
                    let cache_path = hardwave_plugin_host::PluginScanner::default_cache_path();
                    if let Some(ref path) = cache_path {
                        let eng = engine_for_scan.lock();
                        let mut scanner = eng.plugin_scanner.lock();
                        match scanner.load_cache_from_disk(path) {
                            Ok(n) => log::info!("Loaded plugin cache: {n} entries"),
                            Err(e) => log::warn!("Failed to load plugin cache: {e}"),
                        }
                    }
                    {
                        let eng = engine_for_scan.lock();
                        let mut scanner = eng.plugin_scanner.lock();
                        // Register native plugins before scanning external
                        // paths so `find(id)` resolves them as well.
                        scanner
                            .register_natives(hardwave_native_plugins::native_plugin_descriptors());
                        scanner.scan();
                        // Re-register after scan — `scan()` clears the cache
                        // before rebuilding from disk, so natives must be
                        // reapplied to remain discoverable.
                        scanner
                            .register_natives(hardwave_native_plugins::native_plugin_descriptors());
                        if let Some(ref path) = cache_path {
                            if let Err(e) = scanner.save_cache_to_disk(path) {
                                log::warn!("Failed to save plugin cache: {e}");
                            }
                        }
                    }
                });
            }

            // Hot-plug polling removed: cpal's device enumeration takes
            // hundreds of ms and blocks the engine lock, which stalls the
            // meter/transport broadcast and every UI command on a regular
            // cadence. Device loss is still detected via the stream-error
            // recovery path in `poll_audio_health`, and the device list is
            // re-fetched on demand when the Audio Settings panel opens.

            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(33));

                    // Single lock acquisition per tick — coalesces what used to be
                    // four separate engine.lock() calls and eliminates per-tick
                    // contention with command handlers.
                    let (meters, track_payload, transport_payload) = {
                        use std::sync::atomic::Ordering;
                        let mut eng = engine.lock();
                        if let Err(e) = eng.poll_audio_health() {
                            log::error!("Audio health check failed: {e}");
                        }
                        let meters = eng.master_meter();
                        let track_payload: Vec<_> = eng
                            .track_meter_snapshots()
                            .into_iter()
                            .map(|(id, pl, pr, rms, pre_fader)| {
                                serde_json::json!({
                                    "id": id,
                                    "peakL": pl,
                                    "peakR": pr,
                                    "rms": rms,
                                    "preFaderPeak": pre_fader,
                                })
                            })
                            .collect();
                        let pos = eng.transport.position();
                        let playing = eng.transport.is_playing();
                        let bpm = eng.transport.bpm.load(Ordering::Relaxed);
                        let master_db = eng.transport.master_volume_db.load(Ordering::Relaxed);
                        let (num, den) = hardwave_engine::transport::unpack_time_sig(
                            eng.transport.time_sig.load(Ordering::Relaxed),
                        );
                        let pattern_mode = eng.transport.pattern_mode.load(Ordering::Relaxed);
                        let looping = eng.transport.looping.load(Ordering::Relaxed);
                        let loop_start = eng.transport.loop_start.load(Ordering::Relaxed);
                        let loop_end = eng.transport.loop_end.load(Ordering::Relaxed);
                        let transport_payload = serde_json::json!({
                            "position": pos,
                            "playing": playing,
                            "bpm": bpm,
                            "masterVolumeDb": master_db,
                            "timeSig": [num, den],
                            "patternMode": pattern_mode,
                            "looping": looping,
                            "loopStart": loop_start,
                            "loopEnd": loop_end,
                        });
                        (meters, track_payload, transport_payload)
                    };

                    let _ = app_handle.emit("daw:meters", &meters);
                    let _ = app_handle.emit("daw:trackMeters", &track_payload);
                    let _ = app_handle.emit("daw:transport", &transport_payload);
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // Reliable clean-shutdown hook: clear the alive marker so the
                // next launch does not mistake this for a crash.
                if let Ok(dir) = window.app_handle().path().app_cache_dir() {
                    let marker = dir.join("autosaves").join("session.alive");
                    let _ = std::fs::remove_file(marker);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Hardwave DAW");
}
