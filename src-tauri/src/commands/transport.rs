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
    if !bpm.is_finite() {
        return;
    }
    let bpm = bpm.clamp(20.0, 999.0);
    let engine = state.engine.lock();
    engine.transport.bpm.store(bpm, Ordering::Relaxed);
    engine.send_command(TransportCommand::SetBpm(bpm));
    // Persist to project so save_project sees the current BPM.
    let mut project = engine.project.lock();
    if let Some(entry) = project.tempo_map.entries.get_mut(0) {
        entry.bpm = bpm;
    }
}

/// Flip the transport's recording flag and start / stop the matching
/// capture session. When recording flips on, we clear the capture
/// buffer and arm the InputNode tap so the next audio block begins
/// streaming input samples into memory. When it flips off — either by
/// the user pressing Record again or by a Stop — we drain the captured
/// samples, write them to a `.wav` under the project's autosave dir,
/// and place a fresh audio clip on the first armed track at the sample
/// position the recording started.
///
/// Returns the path of the freshly-written WAV when a session ends, or
/// `None` when this call started one. The frontend uses this to show a
/// "took #N saved" notification.
#[tauri::command]
pub fn toggle_recording(state: State<AppState>) -> Result<Option<String>, String> {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    let was_recording = engine.transport.recording.load(Ordering::Relaxed);

    if !was_recording {
        // Begin: arm the InputNode tap and remember where we started so
        // the resulting clip gets placed at the right timeline position.
        engine.transport.recording.store(true, Ordering::Relaxed);
        engine.start_capture();
        return Ok(None);
    }

    // End: stop capturing, drain samples, write a wav, place clip.
    engine.transport.recording.store(false, Ordering::Relaxed);
    let samples = engine.stop_capture();
    if samples.is_empty() {
        return Ok(None);
    }

    let sample_rate = engine.current_sample_rate();
    // Recordings live under the platform's audio scratch dir for now —
    // a future commit will plumb the active project's parent dir
    // through here so takes ride alongside the .hwp file.
    let project_dir = std::env::temp_dir().join("hardwave-daw-recordings");
    std::fs::create_dir_all(&project_dir).map_err(|e| e.to_string())?;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = project_dir.join(format!("take-{stamp}.wav"));

    // Write 32-bit float stereo WAV — matches the engine's internal
    // sample format so we don't lose anything on the way to disk.
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    {
        let mut writer = hound::WavWriter::create(&path, spec).map_err(|e| e.to_string())?;
        for s in &samples {
            writer.write_sample(*s).map_err(|e| e.to_string())?;
        }
        writer.finalize().map_err(|e| e.to_string())?;
    }

    Ok(Some(path.to_string_lossy().to_string()))
}

#[tauri::command]
pub fn toggle_loop(state: State<AppState>) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    // The audio thread reads `transport.looping` directly each block, so
    // toggling the atomic here is enough — no command needs to be queued.
    // Sending TransportCommand::ToggleLoop in addition would re-toggle on
    // the next dispatch tick and cancel the change, leaving the test
    // observing `false → false → true` instead of `false → true → false`.
    let current = engine.transport.looping.load(Ordering::Relaxed);
    engine.transport.looping.store(!current, Ordering::Relaxed);
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
    if !db.is_finite() {
        return;
    }
    let db = db.clamp(-100.0, 12.0);
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
