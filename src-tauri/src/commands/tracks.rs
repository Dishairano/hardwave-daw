use crate::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct TrackInfo {
    id: String,
    name: String,
    kind: String,
    color: String,
    volume_db: f64,
    pan: f64,
    muted: bool,
    soloed: bool,
    solo_safe: bool,
    armed: bool,
    insert_count: usize,
}

fn track_to_info(t: &hardwave_project::Track) -> TrackInfo {
    TrackInfo {
        id: t.id.clone(),
        name: t.name.clone(),
        kind: format!("{:?}", t.kind),
        color: t.color.clone(),
        volume_db: t.volume_db,
        pan: t.pan,
        muted: t.muted,
        soloed: t.soloed,
        solo_safe: t.solo_safe,
        armed: t.armed,
        insert_count: t.inserts.len(),
    }
}

#[tauri::command]
pub fn get_tracks(state: State<AppState>) -> Vec<TrackInfo> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project.tracks.iter().map(track_to_info).collect()
}

#[tauri::command]
pub fn add_audio_track(state: State<AppState>, name: String) -> String {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    project.add_audio_track(name)
}

#[tauri::command]
pub fn add_midi_track(state: State<AppState>, name: String) -> String {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    project.add_midi_track(name)
}

#[tauri::command]
pub fn remove_track(state: State<AppState>, track_id: String) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    project.remove_track(&track_id);
}

#[tauri::command]
pub fn set_track_volume(state: State<AppState>, track_id: String, volume_db: f64) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    if let Some(track) = project.track_mut(&track_id) {
        track.volume_db = volume_db;
    }
}

#[tauri::command]
pub fn set_track_pan(state: State<AppState>, track_id: String, pan: f64) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    if let Some(track) = project.track_mut(&track_id) {
        track.pan = pan;
    }
}

#[tauri::command]
pub fn toggle_mute(state: State<AppState>, track_id: String) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    if let Some(track) = project.track_mut(&track_id) {
        track.muted = !track.muted;
    }
}

#[tauri::command]
pub fn toggle_solo(state: State<AppState>, track_id: String) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    if let Some(track) = project.track_mut(&track_id) {
        track.soloed = !track.soloed;
    }
}

#[tauri::command]
pub fn toggle_solo_safe(state: State<AppState>, track_id: String) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    if let Some(track) = project.track_mut(&track_id) {
        track.solo_safe = !track.solo_safe;
    }
}
