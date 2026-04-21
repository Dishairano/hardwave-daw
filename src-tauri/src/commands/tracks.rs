use crate::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct InsertInfo {
    pub id: String,
    #[serde(rename = "pluginId")]
    pub plugin_id: String,
    #[serde(rename = "pluginName")]
    pub plugin_name: String,
    pub enabled: bool,
    pub wet: f32,
}

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
    inserts: Vec<InsertInfo>,
}

fn track_to_info(
    t: &hardwave_project::Track,
    plugin_name_lookup: &dyn Fn(&str) -> String,
) -> TrackInfo {
    let inserts = t
        .inserts
        .iter()
        .map(|s| InsertInfo {
            id: s.id.clone(),
            plugin_id: s.plugin_id.clone(),
            plugin_name: plugin_name_lookup(&s.plugin_id),
            enabled: s.enabled,
            wet: s.wet,
        })
        .collect();
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
        inserts,
    }
}

#[tauri::command]
pub fn get_tracks(state: State<AppState>) -> Vec<TrackInfo> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    let scanner = engine.plugin_scanner.lock();
    // Build an id → name map once; fall back to the plugin id itself when the
    // plugin is missing from the current scan (uninstalled, or scan not yet
    // run) so the mixer never shows empty slot labels.
    let name_of = |id: &str| -> String {
        scanner
            .find(id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    project
        .tracks
        .iter()
        .map(|t| track_to_info(t, &name_of))
        .collect()
}

#[tauri::command]
pub fn add_audio_track(state: State<AppState>, name: String) -> String {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let id = {
        let mut project = engine.project.lock();
        project.add_audio_track(name)
    };
    engine.sync_track_meters();
    engine.rebuild_graph();
    id
}

#[tauri::command]
pub fn add_midi_track(state: State<AppState>, name: String) -> String {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let id = {
        let mut project = engine.project.lock();
        project.add_midi_track(name)
    };
    engine.sync_track_meters();
    engine.rebuild_graph();
    id
}

#[tauri::command]
pub fn remove_track(state: State<AppState>, track_id: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        project.remove_track(&track_id);
    }
    engine.sync_track_meters();
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_volume(state: State<AppState>, track_id: String, volume_db: f64) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.volume_db = volume_db;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_pan(state: State<AppState>, track_id: String, pan: f64) {
    if !pan.is_finite() {
        return;
    }
    let pan = pan.clamp(-1.0, 1.0);
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.pan = pan;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn toggle_mute(state: State<AppState>, track_id: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.muted = !track.muted;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn toggle_solo(state: State<AppState>, track_id: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.soloed = !track.soloed;
        }
    }
    engine.rebuild_graph();
}

/// Exclusive solo: solo only this track, unsolo all others.
#[tauri::command]
pub fn set_exclusive_solo(state: State<AppState>, track_id: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        let target_currently_soloed = project.track(&track_id).map(|t| t.soloed).unwrap_or(false);

        for track in &mut project.tracks {
            if matches!(track.kind, hardwave_project::track::TrackKind::Master) {
                continue;
            }
            if track.id == track_id {
                // If already the only soloed track, unsolo it (toggle off).
                track.soloed = !target_currently_soloed;
            } else {
                track.soloed = false;
            }
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn toggle_arm(state: State<AppState>, track_id: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.armed = !track.armed;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn reorder_track(state: State<AppState>, track_id: String, new_index: usize) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        // Master must stay last; clamp new_index to non-master range.
        let old_idx = match project.tracks.iter().position(|t| t.id == track_id) {
            Some(i) => i,
            None => return,
        };
        if matches!(
            project.tracks[old_idx].kind,
            hardwave_project::track::TrackKind::Master
        ) {
            return;
        }
        let master_count = project
            .tracks
            .iter()
            .filter(|t| matches!(t.kind, hardwave_project::track::TrackKind::Master))
            .count();
        let max_idx = project.tracks.len().saturating_sub(1 + master_count);
        let target = new_index.min(max_idx);
        let track = project.tracks.remove(old_idx);
        project.tracks.insert(target, track);
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_name(state: State<AppState>, track_id: String, name: String) {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return;
    }
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.name = trimmed.to_string();
        }
    }
}

#[tauri::command]
pub fn set_track_color(state: State<AppState>, track_id: String, color: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.color = color;
        }
    }
}

#[tauri::command]
pub fn toggle_solo_safe(state: State<AppState>, track_id: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.solo_safe = !track.solo_safe;
        }
    }
    engine.rebuild_graph();
}
