use tauri::State;
use crate::AppState;
use hardwave_metering::MeterSnapshot;
use serde::Serialize;

#[derive(Serialize)]
pub struct AudioDeviceInfo {
    name: String,
    is_default: bool,
    sample_rates: Vec<u32>,
    max_channels: u16,
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
    // Access the device manager to list devices
    // For now return empty — device manager is private, will add accessor
    vec![]
}
