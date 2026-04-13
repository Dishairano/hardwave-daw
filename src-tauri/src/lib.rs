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
            // MIDI
            commands::midi::create_midi_clip,
            commands::midi::get_midi_notes,
            commands::midi::add_midi_note,
            commands::midi::update_midi_note,
            commands::midi::delete_midi_note,
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

            std::thread::spawn(move || {
                let mut device_fp: u64 = 0;
                let mut tick: u32 = 0;
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(33));
                    // Poll for device hot-swap once per second.
                    tick = tick.wrapping_add(1);
                    if tick.is_multiple_of(30) {
                        let new_fp = engine.lock().output_device_fingerprint();
                        if new_fp != device_fp {
                            device_fp = new_fp;
                            let _ = app_handle.emit("daw:audioDevicesChanged", &new_fp);
                        }
                    }
                    // Check audio-device health; if the stream failed (e.g. USB unplug),
                    // the engine will transparently restart on the default device.
                    if let Err(e) = engine.lock().poll_audio_health() {
                        log::error!("Audio health check failed: {e}");
                    }
                    let meters = engine.lock().master_meter();
                    let _ = app_handle.emit("daw:meters", &meters);

                    let track_meters = engine.lock().track_meter_snapshots();
                    let track_payload: Vec<_> = track_meters
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
                    let _ = app_handle.emit("daw:trackMeters", &track_payload);

                    let pos;
                    let playing;
                    let bpm;
                    let master_db;
                    let (num, den);
                    let pattern_mode;
                    let looping;
                    let loop_start;
                    let loop_end;
                    {
                        use std::sync::atomic::Ordering;
                        let eng = engine.lock();
                        pos = eng.transport.position();
                        playing = eng.transport.is_playing();
                        bpm = eng.transport.bpm.load(Ordering::Relaxed);
                        master_db = eng.transport.master_volume_db.load(Ordering::Relaxed);
                        (num, den) = hardwave_engine::transport::unpack_time_sig(
                            eng.transport.time_sig.load(Ordering::Relaxed),
                        );
                        pattern_mode = eng.transport.pattern_mode.load(Ordering::Relaxed);
                        looping = eng.transport.looping.load(Ordering::Relaxed);
                        loop_start = eng.transport.loop_start.load(Ordering::Relaxed);
                        loop_end = eng.transport.loop_end.load(Ordering::Relaxed);
                    }
                    let _ = app_handle.emit(
                        "daw:transport",
                        serde_json::json!({
                            "position": pos,
                            "playing": playing,
                            "bpm": bpm,
                            "masterVolumeDb": master_db,
                            "timeSig": [num, den],
                            "patternMode": pattern_mode,
                            "looping": looping,
                            "loopStart": loop_start,
                            "loopEnd": loop_end,
                        }),
                    );
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Hardwave DAW");
}
