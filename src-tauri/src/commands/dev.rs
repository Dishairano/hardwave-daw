//! Commands for the Dev Panel (`packages/daw-ui/src/dev/`), the in-app
//! test harness.
//!
//! They answer only in debug builds and builds with the `dev-tools` feature
//! (the signed test builds); in a release each one refuses. A release used
//! to carry them, and the panel behind Ctrl+Shift+D with them, although
//! this comment said otherwise: the Workspace's hidden panel once exposed
//! data it should not have, and this one can run its test suites on the
//! song that is open.

/// Refuse outside the builds the Dev Panel is for.
fn dev_tools_only() -> Result<(), String> {
    if cfg!(any(debug_assertions, feature = "dev-tools")) {
        Ok(())
    } else {
        Err("the dev tools are not in this build".to_string())
    }
}

use crate::AppState;
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, State};

/// Complete snapshot of every engine atomic the dev panel cares about.
#[derive(Serialize)]
pub struct DevState {
    // Transport atomics
    pub position_samples: u64,
    pub playing: bool,
    pub recording: bool,
    pub looping: bool,
    pub loop_start: u64,
    pub loop_end: u64,
    pub bpm: f64,
    pub master_volume_db: f64,
    pub time_sig_numerator: u32,
    pub time_sig_denominator: u32,
    pub time_sig_packed: u64,
    pub pattern_mode: bool,

    // Audio device
    pub active_device_name: Option<String>,
    pub selected_device_name: Option<String>,
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub stream_running: bool,
    pub stream_error_flag: bool,

    // Master meter (mono summary)
    pub master_peak_db: f32,
    pub master_peak_hold_db: f32,
    pub master_rms_db: f32,
    pub master_true_peak_db: f32,
    pub master_clipped: bool,

    // Per-track meters (post-fader, stereo)
    pub tracks: Vec<DevTrackMeter>,
}

#[derive(Serialize)]
pub struct DevTrackMeter {
    pub id: String,
    pub peak_l_db: f32,
    pub peak_r_db: f32,
    pub rms_db: f32,
    pub pre_fader_peak_db: f32,
}

#[tauri::command]
pub fn dev_dump_state(state: State<AppState>) -> Result<DevState, String> {
    dev_tools_only()?;
    let mut engine = state.engine.lock();
    let t = engine.transport.clone();
    let meter = engine.master_meter();
    let track_meters = engine.track_meter_snapshots();
    let (selected_device, sample_rate, buffer_size) = engine.audio_config();
    let active_device_name = engine
        .audio_device_manager()
        .active_device_name()
        .map(|s| s.to_string());
    let stream_running = engine.is_running();
    let stream_error_flag = engine.audio_device_manager().peek_stream_error();

    let packed = t.time_sig.load(Ordering::Relaxed);
    let (num, den) = hardwave_engine::transport::unpack_time_sig(packed);

    Ok(DevState {
        position_samples: t.position(),
        playing: t.is_playing(),
        recording: t.recording.load(Ordering::Relaxed),
        looping: t.looping.load(Ordering::Relaxed),
        loop_start: t.loop_start.load(Ordering::Relaxed),
        loop_end: t.loop_end.load(Ordering::Relaxed),
        bpm: t.bpm.load(Ordering::Relaxed),
        master_volume_db: t.master_volume_db.load(Ordering::Relaxed),
        time_sig_numerator: num,
        time_sig_denominator: den,
        time_sig_packed: packed,
        pattern_mode: t.pattern_mode.load(Ordering::Relaxed),
        active_device_name,
        selected_device_name: selected_device,
        sample_rate,
        buffer_size,
        stream_running,
        stream_error_flag,
        master_peak_db: meter.peak_db,
        master_peak_hold_db: meter.peak_hold_db,
        master_rms_db: meter.rms_db,
        master_true_peak_db: meter.true_peak_db,
        master_clipped: meter.clipped,
        tracks: track_meters
            .into_iter()
            .map(|(id, pl, pr, rms, pre_fader)| DevTrackMeter {
                id,
                peak_l_db: pl,
                peak_r_db: pr,
                rms_db: rms,
                pre_fader_peak_db: pre_fader,
            })
            .collect(),
    })
}

/// Flip the audio-device stream_error atomic so the engine's health-poll
/// recovery path runs for real.
#[tauri::command]
pub fn dev_force_device_error(state: State<AppState>) -> Result<(), String> {
    dev_tools_only()?;
    let engine = state.engine.lock();
    engine.audio_device_manager().inject_stream_error();
    Ok(())
}

/// Resolve a bundled test-asset path to an absolute filesystem path that
/// `import_audio_file` can load.
#[tauri::command]
pub fn dev_resolve_test_asset(app: AppHandle, name: String) -> Result<String, String> {
    dev_tools_only()?;
    let path = app
        .path()
        .resolve(
            format!("test_assets/{}", name),
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| format!("failed to resolve test asset '{}': {}", name, e))?;
    path.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "test asset path is not valid UTF-8".to_string())
}

/// List the test assets bundled with the dev build.
#[tauri::command]
pub fn dev_list_test_assets() -> Result<Vec<&'static str>, String> {
    dev_tools_only()?;
    Ok(vec![
        "sine_1khz_-6dbfs_stereo_5s.wav",
        "pink_noise_-12dbfs_10s.wav",
        "tone_burst_silence.wav",
        "stereo_pan_test.wav",
        "sine-440-1s.wav",
        "sine_44100.wav",
    ])
}
