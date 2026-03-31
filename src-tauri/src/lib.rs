use std::sync::Arc;
use parking_lot::Mutex;
use tauri::{Manager, Emitter};

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
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            // Transport
            commands::transport::play,
            commands::transport::stop,
            commands::transport::set_position,
            commands::transport::set_bpm,
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
            // Audio
            commands::audio::import_audio_file,
            commands::audio::get_track_clips,
        ])
        .setup(|app| {
            log::info!("Hardwave DAW starting");

            // Start meter broadcast thread
            let state = app.state::<AppState>();
            let engine = Arc::clone(&state.engine);
            let app_handle = app.handle().clone();

            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(33));
                    let meters = engine.lock().master_meter();
                    let _ = app_handle.emit("daw:meters", &meters);

                    let pos;
                    let playing;
                    let bpm;
                    {
                        let eng = engine.lock();
                        pos = eng.transport.position();
                        playing = eng.transport.is_playing();
                        bpm = eng.transport.bpm.load(std::sync::atomic::Ordering::Relaxed);
                    }
                    let _ = app_handle.emit("daw:transport", serde_json::json!({
                        "position": pos,
                        "playing": playing,
                        "bpm": bpm,
                    }));
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Hardwave DAW");
}
