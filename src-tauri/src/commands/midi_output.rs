use crate::AppState;
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::State;

#[derive(Serialize)]
pub struct MidiClockStatus {
    pub enabled: bool,
    pub open_ports: Vec<String>,
}

#[tauri::command]
pub fn list_midi_outputs(state: State<AppState>) -> Vec<String> {
    let manager = state.midi_clock.output.lock();
    manager.list_ports()
}

#[tauri::command]
pub fn open_midi_output(state: State<AppState>, port_name: String) -> Result<(), String> {
    let mut manager = state.midi_clock.output.lock();
    manager.open(&port_name)
}

#[tauri::command]
pub fn close_midi_output(state: State<AppState>, port_name: String) {
    let mut manager = state.midi_clock.output.lock();
    manager.close(&port_name);
}

#[tauri::command]
pub fn set_midi_clock_enabled(state: State<AppState>, enabled: bool) {
    state.midi_clock.enabled.store(enabled, Ordering::Relaxed);
}

#[tauri::command]
pub fn get_midi_clock_status(state: State<AppState>) -> MidiClockStatus {
    let enabled = state.midi_clock.enabled.load(Ordering::Relaxed);
    let open_ports = state.midi_clock.output.lock().open_port_names();
    MidiClockStatus {
        enabled,
        open_ports,
    }
}
