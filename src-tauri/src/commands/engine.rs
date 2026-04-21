use crate::AppState;
use hardwave_metering::MeterSnapshot;
use serde::Serialize;
use tauri::State;

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
    state.engine.lock().set_wasapi_exclusive(enabled)
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
        .set_audio_config(device, sample_rate, buffer_size)
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
pub fn set_audio_input_config(
    state: State<AppState>,
    device: Option<String>,
    channels: u16,
) {
    state.engine.lock().set_input_config(device, channels);
}
