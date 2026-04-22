use crate::midi_map::{MidiMapTarget, MidiMapping};
use crate::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct MidiLearnStatus {
    pub learning: bool,
    pub target: Option<MidiMapTarget>,
    pub last_learned: Option<MidiMapping>,
}

#[tauri::command]
pub fn midi_learn_start(state: State<AppState>, target: MidiMapTarget) {
    let mut m = state.midi_mappings.lock();
    m.last_learned = None;
    m.learn = Some(target);
}

#[tauri::command]
pub fn midi_learn_cancel(state: State<AppState>) {
    state.midi_mappings.lock().learn = None;
}

#[tauri::command]
pub fn midi_learn_status(state: State<AppState>) -> MidiLearnStatus {
    let m = state.midi_mappings.lock();
    MidiLearnStatus {
        learning: m.learn.is_some(),
        target: m.learn.clone(),
        last_learned: m.last_learned.clone(),
    }
}

#[tauri::command]
pub fn list_midi_mappings(state: State<AppState>) -> Vec<MidiMapping> {
    state.midi_mappings.lock().mappings.clone()
}

#[tauri::command]
pub fn remove_midi_mapping(state: State<AppState>, id: u32) {
    let mut m = state.midi_mappings.lock();
    if m.remove(id) {
        m.save();
    }
}

#[tauri::command]
pub fn clear_midi_mappings(state: State<AppState>) {
    let mut m = state.midi_mappings.lock();
    m.clear();
    m.save();
}
