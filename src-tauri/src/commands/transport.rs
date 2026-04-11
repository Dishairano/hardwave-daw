use crate::AppState;
use hardwave_engine::TransportCommand;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct TransportInfo {
    playing: bool,
    recording: bool,
    looping: bool,
    position_samples: u64,
    bpm: f64,
    loop_start: u64,
    loop_end: u64,
}

#[tauri::command]
pub fn play(state: State<AppState>) {
    state.engine.lock().send_command(TransportCommand::Play);
}

#[tauri::command]
pub fn stop(state: State<AppState>) {
    state.engine.lock().send_command(TransportCommand::Stop);
}

#[tauri::command]
pub fn set_position(state: State<AppState>, position: u64) {
    state
        .engine
        .lock()
        .send_command(TransportCommand::SetPosition(position));
}

#[tauri::command]
pub fn set_bpm(state: State<AppState>, bpm: f64) {
    state
        .engine
        .lock()
        .send_command(TransportCommand::SetBpm(bpm));
}

#[tauri::command]
pub fn toggle_loop(state: State<AppState>) {
    state
        .engine
        .lock()
        .send_command(TransportCommand::ToggleLoop);
}

#[tauri::command]
pub fn set_loop(state: State<AppState>, start: u64, end: u64) {
    state
        .engine
        .lock()
        .send_command(TransportCommand::SetLoop(start, end));
}

#[tauri::command]
pub fn get_transport_state(state: State<AppState>) -> TransportInfo {
    let engine = state.engine.lock();
    let t = &engine.transport;
    TransportInfo {
        playing: t.is_playing(),
        recording: t.recording.load(std::sync::atomic::Ordering::Relaxed),
        looping: t.looping.load(std::sync::atomic::Ordering::Relaxed),
        position_samples: t.position(),
        bpm: t.bpm.load(std::sync::atomic::Ordering::Relaxed),
        loop_start: t.loop_start.load(std::sync::atomic::Ordering::Relaxed),
        loop_end: t.loop_end.load(std::sync::atomic::Ordering::Relaxed),
    }
}
