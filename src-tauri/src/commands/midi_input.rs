use crate::AppState;
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::State;

#[derive(Serialize)]
pub struct MidiActivitySnapshot {
    pub open_ports: Vec<String>,
    pub ms_since_last_event: Option<u64>,
}

#[derive(Serialize)]
pub struct MidiClockSyncStatus {
    pub enabled: bool,
    pub ticks_seen: bool,
    pub last_bpm: Option<f64>,
}

#[tauri::command]
pub fn list_midi_inputs(state: State<AppState>) -> Vec<String> {
    let engine = state.engine.lock();
    let manager = engine.midi_input.lock();
    manager.list_ports()
}

#[tauri::command]
pub fn open_midi_input(state: State<AppState>, port_name: String) -> Result<(), String> {
    let engine = state.engine.lock();
    let mut manager = engine.midi_input.lock();
    manager.open(&port_name)
}

#[tauri::command]
pub fn close_midi_input(state: State<AppState>, port_name: String) {
    let engine = state.engine.lock();
    let mut manager = engine.midi_input.lock();
    manager.close(&port_name);
}

#[tauri::command]
pub fn close_all_midi_inputs(state: State<AppState>) {
    let engine = state.engine.lock();
    let mut manager = engine.midi_input.lock();
    manager.close_all();
}

#[tauri::command]
pub fn get_midi_activity(state: State<AppState>) -> MidiActivitySnapshot {
    let engine = state.engine.lock();
    let manager = engine.midi_input.lock();
    MidiActivitySnapshot {
        open_ports: manager.open_port_names(),
        ms_since_last_event: manager.ms_since_last_event(),
    }
}

#[tauri::command]
pub fn get_midi_desired_ports(state: State<AppState>) -> Vec<String> {
    let engine = state.engine.lock();
    let manager = engine.midi_input.lock();
    manager.desired_port_names()
}

#[tauri::command]
pub fn set_midi_clock_sync_enabled(state: State<AppState>, enabled: bool) {
    state.midi_sync.enabled.store(enabled, Ordering::Relaxed);
}

#[tauri::command]
pub fn get_midi_clock_sync_status(state: State<AppState>) -> MidiClockSyncStatus {
    MidiClockSyncStatus {
        enabled: state.midi_sync.enabled.load(Ordering::Relaxed),
        ticks_seen: state.midi_sync.ticks_seen.load(Ordering::Relaxed),
        last_bpm: *state.midi_sync.last_bpm.lock(),
    }
}
