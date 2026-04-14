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
    master_volume_db: f64,
    time_sig_numerator: u32,
    time_sig_denominator: u32,
    pattern_mode: bool,
}

#[tauri::command]
pub fn play(state: State<AppState>) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine.transport.playing.store(true, Ordering::Relaxed);
    engine.send_command(TransportCommand::Play);
}

#[tauri::command]
pub fn stop(state: State<AppState>) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    let was_playing = engine.transport.playing.swap(false, Ordering::Relaxed);
    if !was_playing {
        let loop_start = if engine.transport.looping.load(Ordering::Relaxed) {
            engine.transport.loop_start.load(Ordering::Relaxed)
        } else {
            0
        };
        engine.transport.set_position(loop_start);
    }
    engine.transport.recording.store(false, Ordering::Relaxed);
    engine.send_command(TransportCommand::Stop);
}

#[tauri::command]
pub fn set_position(state: State<AppState>, position: u64) {
    let engine = state.engine.lock();
    engine.transport.set_position(position);
    // Also queue for the audio thread so double-stop logic stays consistent.
    engine.send_command(TransportCommand::SetPosition(position));
}

#[tauri::command]
pub fn set_bpm(state: State<AppState>, bpm: f64) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine.transport.bpm.store(bpm, Ordering::Relaxed);
    engine.send_command(TransportCommand::SetBpm(bpm));
    // Persist to project so save_project sees the current BPM.
    let mut project = engine.project.lock();
    if let Some(entry) = project.tempo_map.entries.get_mut(0) {
        entry.bpm = bpm;
    }
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
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine.transport.loop_start.store(start, Ordering::Relaxed);
    engine.transport.loop_end.store(end, Ordering::Relaxed);
    engine.send_command(TransportCommand::SetLoop(start, end));
}

#[tauri::command]
pub fn set_master_volume(state: State<AppState>, db: f64) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine
        .transport
        .master_volume_db
        .store(db, Ordering::Relaxed);
    engine.send_command(TransportCommand::SetMasterVolume(db));
}

#[tauri::command]
pub fn set_time_signature(state: State<AppState>, numerator: u32, denominator: u32) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine.transport.time_sig.store(
        hardwave_engine::transport::pack_time_sig(numerator, denominator),
        Ordering::Relaxed,
    );
    engine.send_command(TransportCommand::SetTimeSignature(numerator, denominator));
}

#[tauri::command]
pub fn set_pattern_mode(state: State<AppState>, enabled: bool) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine
        .transport
        .pattern_mode
        .store(enabled, Ordering::Relaxed);
    engine.send_command(TransportCommand::SetPatternMode(enabled));
}

#[tauri::command]
pub fn get_transport_state(state: State<AppState>) -> TransportInfo {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    let t = &engine.transport;
    let (num, den) =
        hardwave_engine::transport::unpack_time_sig(t.time_sig.load(Ordering::Relaxed));
    TransportInfo {
        playing: t.is_playing(),
        recording: t.recording.load(Ordering::Relaxed),
        looping: t.looping.load(Ordering::Relaxed),
        position_samples: t.position(),
        bpm: t.bpm.load(Ordering::Relaxed),
        loop_start: t.loop_start.load(Ordering::Relaxed),
        loop_end: t.loop_end.load(Ordering::Relaxed),
        master_volume_db: t.master_volume_db.load(Ordering::Relaxed),
        time_sig_numerator: num,
        time_sig_denominator: den,
        pattern_mode: t.pattern_mode.load(Ordering::Relaxed),
    }
}
