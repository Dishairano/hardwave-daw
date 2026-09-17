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
    /// Armed-and-parked: Play/Record pressed with wait-for-input on,
    /// playback starts on the first MIDI event.
    waiting_for_input: bool,
}

#[tauri::command]
pub fn play(state: State<AppState>) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    // Wait-for-input (FL Ctrl+I): park instead of starting — the audio
    // thread flips `playing` on the first MIDI event. The engine-side
    // TransportCommand handler applies the same rule, so both paths
    // agree regardless of which runs first.
    if engine.transport.wait_for_input.load(Ordering::Relaxed)
        && !engine.transport.playing.load(Ordering::Relaxed)
    {
        engine.transport.wait_pending.store(true, Ordering::Relaxed);
    } else {
        engine.transport.playing.store(true, Ordering::Relaxed);
    }
    engine.send_command(TransportCommand::Play);
}

/// Toggle FL-style "wait for input" — with it on, Play/Record park the
/// transport until the first MIDI event arrives.
#[tauri::command]
pub fn set_wait_for_input(state: State<AppState>, enabled: bool) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine
        .transport
        .wait_for_input
        .store(enabled, Ordering::Relaxed);
    // Disabling while parked honours the earlier Play press.
    if !enabled && engine.transport.wait_pending.swap(false, Ordering::Relaxed) {
        engine.transport.playing.store(true, Ordering::Relaxed);
    }
    engine.send_command(TransportCommand::SetWaitForInput(enabled));
}

#[tauri::command]
pub fn stop(state: State<AppState>) -> Result<Option<String>, String> {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine
        .transport
        .wait_pending
        .store(false, Ordering::Relaxed);
    // Stop during a count-in cancels it. Without this the count would finish
    // in the background and start playing after the user had already stopped.
    engine
        .transport
        .count_in_then_play
        .store(false, Ordering::Relaxed);
    engine
        .transport
        .count_in_remaining
        .store(0, Ordering::Relaxed);
    engine.transport.count_in_total.store(0, Ordering::Relaxed);
    let was_playing = engine.transport.playing.swap(false, Ordering::Relaxed);
    if !was_playing {
        let loop_start = if engine.transport.looping.load(Ordering::Relaxed) {
            engine.transport.loop_start.load(Ordering::Relaxed)
        } else {
            0
        };
        engine.transport.set_position(loop_start);
    }
    // Stop also has to finalise an in-flight recording: previously this
    // flipped the recording flag without draining the capture buffer,
    // so pressing Space (which calls stop) mid-record silently dropped
    // the take. Now we drain, write the WAV, and return its path so the
    // frontend can place the clip on the armed track — mirroring how
    // toggle_recording does it on the trailing edge.
    let was_recording = engine.transport.recording.swap(false, Ordering::Relaxed);
    engine.send_command(TransportCommand::Stop);
    if was_recording {
        finalize_recording_session(&engine)
    } else {
        Ok(None)
    }
}

/// Drain the engine's capture buffer and write a fresh WAV to the
/// recordings scratch dir. Returns the file path on success or `None`
/// when no samples were captured. Shared by `stop` and the trailing
/// edge of `toggle_recording` so the two paths can't drift.
fn finalize_recording_session(
    engine: &hardwave_engine::DawEngine,
) -> Result<Option<String>, String> {
    let samples = engine.stop_capture();
    if samples.is_empty() {
        return Ok(None);
    }

    let sample_rate = engine.current_sample_rate();
    let project_dir = std::env::temp_dir().join("hardwave-daw-recordings");
    std::fs::create_dir_all(&project_dir).map_err(|e| e.to_string())?;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = project_dir.join(format!("take-{stamp}.wav"));

    // 32-bit float stereo WAV matches the engine's internal sample format.
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
pub fn set_position(state: State<AppState>, position: u64) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine.transport.set_position(position);
    // Also queue for the audio thread so double-stop logic stays consistent.
    engine.send_command(TransportCommand::SetPosition(position));

    // Tempo and signature at the new position. While playing the audio thread
    // follows the map itself, but a seek while stopped never reaches it, so
    // the transport read-out and the click kept the values from wherever the
    // playhead used to be.
    let (bpm, num, den) = {
        let project = engine.project.lock();
        if project.tempo_map.entries.len() < 2 {
            return;
        }
        let tick = project
            .tempo_map
            .samples_to_tick(position, engine.current_sample_rate() as f64);
        let (num, den) = project.tempo_map.time_sig_at(tick);
        (project.tempo_map.bpm_at(tick), num, den)
    };
    engine.transport.bpm.store(bpm, Ordering::Relaxed);
    engine.transport.time_sig.store(
        hardwave_engine::transport::pack_time_sig(num, den),
        Ordering::Relaxed,
    );
    engine.send_command(TransportCommand::SetTimeSignature(num, den));
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

    // End: stop capturing, drain samples, write a WAV, return its path.
    engine.transport.recording.store(false, Ordering::Relaxed);
    finalize_recording_session(&engine)
}

/// Cancel an in-flight recording without finalising the take. FL
/// Studio's Tools → Macros → Panic → Cancel recording — drops the
/// captured samples on the floor instead of writing the WAV, so the
/// user can bail from a recording mistake without leaving an
/// orphaned take on disk or in the project.
///
/// No-op when not currently recording.
#[tauri::command]
pub fn cancel_recording(state: State<AppState>) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    let was_recording = engine.transport.recording.swap(false, Ordering::Relaxed);
    if !was_recording {
        return;
    }
    // Drain and drop — same path as the regular stop, just no WAV write.
    let _ = engine.stop_capture();
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

/// Set the project's time signature.
///
/// This used to write the transport atomics only, so the signature was lost
/// on save: reopening the project put it back to whatever the tempo map said,
/// which was 4/4 for every project ever made in this DAW. It now writes the
/// tempo map as well.
///
/// Entries later in the song that carry the old signature follow the change,
/// because they inherited it rather than being set deliberately. An entry
/// with a different signature is a deliberate mid-song change and is left
/// alone.
#[tauri::command]
pub fn set_time_signature(
    state: State<AppState>,
    numerator: u32,
    denominator: u32,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let (num, den) = crate::commands::project::validate_time_signature(numerator, denominator)?;
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    {
        let mut project = engine.project.lock();
        let previous = project.tempo_map.time_sig_at(0);
        for entry in project.tempo_map.entries.iter_mut() {
            if (entry.time_sig_num, entry.time_sig_den) == previous {
                entry.time_sig_num = num;
                entry.time_sig_den = den;
            }
        }
    }
    engine.transport.time_sig.store(
        hardwave_engine::transport::pack_time_sig(num, den),
        Ordering::Relaxed,
    );
    engine.send_command(TransportCommand::SetTimeSignature(num, den));
    Ok(())
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
        waiting_for_input: t.wait_pending.load(Ordering::Relaxed),
    }
}

/// Count off `bars` before playback starts.
///
/// The count-in used to be a row of WebAudio oscillators in the WebView plus a
/// `setTimeout` that called play when it thought they had finished, so the
/// clicks a take was counted in against came from a different clock than the
/// audio, and playback began wherever the timeout landed. The audio thread now
/// owns both: it counts in samples and starts playback on the sample the count
/// ends.
#[tauri::command]
pub fn start_count_in(state: State<AppState>, bars: u32) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    if bars == 0 {
        engine.transport.playing.store(true, Ordering::Relaxed);
        return;
    }
    let bpm = engine.transport.bpm.load(Ordering::Relaxed).max(1.0);
    let sample_rate = engine.transport.sample_rate.load(Ordering::Relaxed).max(1) as f64;
    let (beats_per_bar, den) = hardwave_engine::transport::unpack_time_sig(
        engine.transport.time_sig.load(Ordering::Relaxed),
    );
    let beats = (bars * beats_per_bar.max(1)) as f64;
    // A beat is a note value, so the denominator sets its length. Counting a
    // bar of 7/8 in quarter notes made the count-in twice as long as the bar
    // it was counting in.
    let samples = (beats * 60.0 / bpm * sample_rate * 4.0 / den.max(1) as f64).round() as u64;

    engine.transport.playing.store(false, Ordering::Relaxed);
    engine
        .transport
        .count_in_total
        .store(samples, Ordering::Relaxed);
    engine
        .transport
        .count_in_remaining
        .store(samples, Ordering::Relaxed);
    engine
        .transport
        .count_in_then_play
        .store(true, Ordering::Relaxed);
}

/// How far a count-in has got, for the on-screen counter.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountInState {
    pub active: bool,
    /// Beats already counted, and the total, so the UI can show "2 of 4".
    pub beat: u32,
    pub total_beats: u32,
}

#[tauri::command]
pub fn get_count_in_state(state: State<AppState>) -> CountInState {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    let remaining = engine.transport.count_in_remaining.load(Ordering::Relaxed);
    let total = engine.transport.count_in_total.load(Ordering::Relaxed);
    if total == 0 {
        return CountInState {
            active: false,
            beat: 0,
            total_beats: 0,
        };
    }
    let bpm = engine.transport.bpm.load(Ordering::Relaxed).max(1.0);
    let sample_rate = engine.transport.sample_rate.load(Ordering::Relaxed).max(1) as f64;
    let samples_per_beat = (60.0 / bpm * sample_rate).max(1.0);
    let elapsed = total.saturating_sub(remaining) as f64;
    CountInState {
        active: remaining > 0,
        beat: (elapsed / samples_per_beat).floor() as u32,
        total_beats: (total as f64 / samples_per_beat).round() as u32,
    }
}
