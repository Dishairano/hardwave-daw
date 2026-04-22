use crate::{AppState, AudioPrefs};
use hardwave_metering::MeterSnapshot;
use serde::Serialize;
use tauri::State;

/// Snapshot the current engine audio config into a pref struct and write it to
/// disk. Called from every setter so the on-disk prefs stay aligned with the
/// running engine state.
fn persist_audio_prefs(state: &State<AppState>) {
    let engine = state.engine.lock();
    let (output_device, sample_rate, buffer_size) = engine.audio_config();
    let (input_device, input_channels) = engine.input_config();
    let prefs = AudioPrefs {
        output_device,
        sample_rate,
        buffer_size,
        wasapi_exclusive: engine.wasapi_exclusive(),
        input_device,
        input_channels,
    };
    drop(engine);
    prefs.save();
}

#[derive(Serialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default: bool,
    pub sample_rates: Vec<u32>,
    pub max_channels: u16,
}

#[derive(Serialize)]
pub struct AudioConfig {
    pub device: Option<String>,
    pub sample_rate: u32,
    pub buffer_size: u32,
}

#[tauri::command]
pub fn start_engine(state: State<AppState>) -> Result<(), String> {
    state.engine.lock().start()
}

#[tauri::command]
pub fn stop_engine(state: State<AppState>) {
    state.engine.lock().stop();
}

#[tauri::command]
pub fn get_meters(state: State<AppState>) -> MeterSnapshot {
    state.engine.lock().master_meter()
}

/// Return the most recent `n_frames` stereo frames from the master bus,
/// interleaved L,R,L,R… The UI uses this for oscilloscope / correlation /
/// spectrum visualizations. `n_frames` is clamped to the tap capacity.
#[tauri::command]
pub fn get_master_samples(state: State<AppState>, n_frames: u32) -> Vec<f32> {
    state.engine.lock().master_tap_snapshot(n_frames as usize)
}

#[tauri::command]
pub fn get_audio_devices(state: State<AppState>) -> Vec<AudioDeviceInfo> {
    let engine = state.engine.lock();
    engine
        .audio_device_manager()
        .list_output_devices()
        .into_iter()
        .map(|d| AudioDeviceInfo {
            name: d.name,
            is_default: d.is_default,
            sample_rates: d.sample_rates,
            max_channels: d.max_channels,
        })
        .collect()
}

#[tauri::command]
pub fn get_audio_config(state: State<AppState>) -> AudioConfig {
    let (device, sample_rate, buffer_size) = state.engine.lock().audio_config();
    AudioConfig {
        device,
        sample_rate,
        buffer_size,
    }
}

#[tauri::command]
pub fn list_audio_hosts() -> Vec<String> {
    hardwave_engine::DawEngine::list_audio_hosts()
}

#[tauri::command]
pub fn get_audio_host(state: State<AppState>) -> String {
    state.engine.lock().audio_host_name()
}

#[tauri::command]
pub fn set_audio_host(state: State<AppState>, host_name: String) -> Result<(), String> {
    state.engine.lock().set_audio_host(&host_name)
}

#[derive(Serialize)]
pub struct WasapiExclusiveStatus {
    pub enabled: bool,
    pub available: bool,
}

#[tauri::command]
pub fn get_wasapi_exclusive(state: State<AppState>) -> WasapiExclusiveStatus {
    let engine = state.engine.lock();
    WasapiExclusiveStatus {
        enabled: engine.wasapi_exclusive(),
        available: engine.wasapi_exclusive_available(),
    }
}

#[tauri::command]
pub fn set_wasapi_exclusive(state: State<AppState>, enabled: bool) -> Result<(), String> {
    state.engine.lock().set_wasapi_exclusive(enabled)?;
    persist_audio_prefs(&state);
    Ok(())
}

#[tauri::command]
pub fn set_audio_config(
    state: State<AppState>,
    device: Option<String>,
    sample_rate: u32,
    buffer_size: u32,
) -> Result<(), String> {
    state
        .engine
        .lock()
        .set_audio_config(device, sample_rate, buffer_size)?;
    persist_audio_prefs(&state);
    Ok(())
}

#[tauri::command]
pub fn get_audio_input_devices(state: State<AppState>) -> Vec<AudioDeviceInfo> {
    let engine = state.engine.lock();
    engine
        .audio_device_manager()
        .list_input_devices()
        .into_iter()
        .map(|d| AudioDeviceInfo {
            name: d.name,
            is_default: d.is_default,
            sample_rates: d.sample_rates,
            max_channels: d.max_channels,
        })
        .collect()
}

#[derive(Serialize)]
pub struct AudioInputConfig {
    pub device: Option<String>,
    pub channels: u16,
}

#[tauri::command]
pub fn get_audio_input_config(state: State<AppState>) -> AudioInputConfig {
    let (device, channels) = state.engine.lock().input_config();
    AudioInputConfig { device, channels }
}

#[tauri::command]
pub fn set_audio_input_config(state: State<AppState>, device: Option<String>, channels: u16) {
    state.engine.lock().set_input_config(device, channels);
    persist_audio_prefs(&state);
}

#[derive(Serialize)]
pub struct InputMeterSnapshot {
    pub peak_l: f32,
    pub peak_r: f32,
    pub running: bool,
    pub sample_rate: u32,
    pub buffer_size: u32,
}

#[tauri::command]
pub fn start_input_monitoring(state: State<AppState>) -> Result<(), String> {
    state.engine.lock().start_input_monitoring()
}

#[tauri::command]
pub fn stop_input_monitoring(state: State<AppState>) {
    state.engine.lock().stop_input_monitoring();
}

#[tauri::command]
pub fn set_direct_monitoring(state: State<AppState>, enabled: bool) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine
        .transport
        .direct_monitoring
        .store(enabled, Ordering::Relaxed);
    engine.rebuild_graph();
}

#[tauri::command]
pub fn get_direct_monitoring(state: State<AppState>) -> bool {
    use std::sync::atomic::Ordering;
    state
        .engine
        .lock()
        .transport
        .direct_monitoring
        .load(Ordering::Relaxed)
}

#[derive(Serialize)]
pub struct GraphLatency {
    pub samples: u32,
    pub ms: f64,
    #[serde(rename = "pdcEnabled")]
    pub pdc_enabled: bool,
}

#[tauri::command]
pub fn get_graph_latency(state: State<AppState>) -> GraphLatency {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    let pdc_enabled = engine.transport.pdc_enabled.load(Ordering::Relaxed);
    let raw = engine.graph_latency_samples.load(Ordering::Relaxed);
    let samples = if pdc_enabled { raw } else { 0 };
    let (_, sample_rate, _) = engine.audio_config();
    let ms = if sample_rate > 0 {
        samples as f64 / sample_rate as f64 * 1000.0
    } else {
        0.0
    };
    GraphLatency {
        samples,
        ms,
        pdc_enabled,
    }
}

#[tauri::command]
pub fn get_pdc_enabled(state: State<AppState>) -> bool {
    use std::sync::atomic::Ordering;
    state
        .engine
        .lock()
        .transport
        .pdc_enabled
        .load(Ordering::Relaxed)
}

#[tauri::command]
pub fn set_pdc_enabled(state: State<AppState>, enabled: bool) {
    use std::sync::atomic::Ordering;
    state
        .engine
        .lock()
        .transport
        .pdc_enabled
        .store(enabled, Ordering::Relaxed);
}

#[tauri::command]
pub fn get_input_meter(state: State<AppState>) -> InputMeterSnapshot {
    let engine = state.engine.lock();
    let running = engine.is_input_monitoring();
    let (peak_l, peak_r) = if running {
        engine.input_peak_snapshot()
    } else {
        (0.0, 0.0)
    };
    let (sample_rate, buffer_size) = engine.input_active_config();
    InputMeterSnapshot {
        peak_l,
        peak_r,
        running,
        sample_rate,
        buffer_size,
    }
}
