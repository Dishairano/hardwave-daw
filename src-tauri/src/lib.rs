use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{Emitter, Manager};

mod commands;

use hardwave_engine::DawEngine;

/// Shared engine state accessible from Tauri commands.
pub struct AppState {
    pub engine: Arc<Mutex<DawEngine>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    let engine = DawEngine::new();
    let state = AppState {
        engine: Arc::new(Mutex::new(engine)),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
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
            commands::tracks::remove_track,
            commands::tracks::set_track_volume,
            commands::tracks::set_track_pan,
            commands::tracks::toggle_mute,
            commands::tracks::toggle_solo,
            commands::tracks::set_exclusive_solo,
            commands::tracks::toggle_solo_safe,
            commands::tracks::toggle_arm,
            commands::tracks::reorder_track,
            // Plugins
            commands::plugins::scan_plugins,
            commands::plugins::get_plugins,
            commands::plugins::add_plugin_to_track,
            commands::plugins::remove_plugin_from_track,
            // Project
            commands::project::new_project,
            commands::project::save_project,
            commands::project::load_project,
            commands::project::get_project_info,
            // Engine
            commands::engine::start_engine,
            commands::engine::stop_engine,
            commands::engine::get_meters,
            commands::engine::get_audio_devices,
            commands::engine::get_audio_config,
            commands::engine::set_audio_config,
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
            commands::audio::toggle_clip_reverse,
            commands::audio::set_clip_pitch,
            commands::audio::set_clip_stretch,
            // MIDI
            commands::midi::create_midi_clip,
            commands::midi::get_midi_notes,
            commands::midi::add_midi_note,
            commands::midi::update_midi_note,
            commands::midi::delete_midi_note,
            // Undo/redo
            commands::history::undo,
            commands::history::redo,
            commands::history::history_sizes,
            // Dev panel (stripped before merge to master)
            commands::dev::dev_dump_state,
            commands::dev::dev_force_device_error,
            commands::dev::dev_resolve_test_asset,
            commands::dev::dev_list_test_assets,
        ])
        .setup(|app| {
            log::info!("Hardwave DAW starting");

            // Start meter broadcast thread
            let state = app.state::<AppState>();
            let engine = Arc::clone(&state.engine);
            let app_handle = app.handle().clone();

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
        .run(tauri::generate_context!())
        .expect("error while running Hardwave DAW");
}
