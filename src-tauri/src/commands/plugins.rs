use crate::AppState;
use hardwave_plugin_host::scanner::ScanDiff;
use hardwave_plugin_host::PluginDescriptor;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub fn scan_plugins(app: AppHandle, state: State<AppState>) -> Vec<PluginDescriptor> {
    let engine = state.engine.lock();
    let mut scanner = engine.plugin_scanner.lock();
    let emitter = app.clone();
    let progress: hardwave_plugin_host::scanner::ScanProgress = Box::new(move |count, label| {
        let _ = emitter.emit(
            "daw:pluginScanProgress",
            serde_json::json!({ "count": count, "current": label }),
        );
    });
    let result = scanner.scan_with_progress(Some(progress)).to_vec();
    let _ = app.emit(
        "daw:pluginScanComplete",
        serde_json::json!({ "count": result.len() }),
    );
    if let Some(path) = hardwave_plugin_host::PluginScanner::default_cache_path() {
        if let Err(e) = scanner.save_cache_to_disk(&path) {
            log::warn!("Failed to persist plugin cache: {e}");
        }
    }
    result
}

#[tauri::command]
pub fn get_plugins(state: State<AppState>) -> Vec<PluginDescriptor> {
    let engine = state.engine.lock();
    let scanner = engine.plugin_scanner.lock();
    scanner.plugins().to_vec()
}

#[tauri::command]
pub fn get_last_scan_diff(state: State<AppState>) -> ScanDiff {
    let engine = state.engine.lock();
    let scanner = engine.plugin_scanner.lock();
    scanner.last_diff().clone()
}

#[tauri::command]
pub fn get_plugin_blocklist(state: State<AppState>) -> Vec<String> {
    let engine = state.engine.lock();
    let scanner = engine.plugin_scanner.lock();
    let mut list: Vec<String> = scanner.blocklist.iter().cloned().collect();
    list.sort();
    list
}

#[tauri::command]
pub fn set_plugin_blocklist(state: State<AppState>, ids: Vec<String>) {
    let engine = state.engine.lock();
    let mut scanner = engine.plugin_scanner.lock();
    scanner.blocklist = ids.into_iter().collect();
}

#[tauri::command]
pub fn get_custom_scan_paths(state: State<AppState>) -> (Vec<String>, Vec<String>) {
    let engine = state.engine.lock();
    let scanner = engine.plugin_scanner.lock();
    let vst3 = scanner
        .custom_vst3_paths
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let clap = scanner
        .custom_clap_paths
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    (vst3, clap)
}

#[tauri::command]
pub fn set_custom_scan_paths(state: State<AppState>, vst3: Vec<String>, clap: Vec<String>) {
    let engine = state.engine.lock();
    let mut scanner = engine.plugin_scanner.lock();
    scanner.custom_vst3_paths = vst3.into_iter().map(PathBuf::from).collect();
    scanner.custom_clap_paths = clap.into_iter().map(PathBuf::from).collect();
}

#[tauri::command]
pub fn plugin_cache_path() -> Option<String> {
    hardwave_plugin_host::PluginScanner::default_cache_path().map(|p| p.display().to_string())
}

#[tauri::command]
pub fn add_plugin_to_track(
    state: State<AppState>,
    track_id: String,
    plugin_id: String,
) -> Result<String, String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let scanner = engine.plugin_scanner.lock();

    let descriptor = scanner
        .find(&plugin_id)
        .ok_or_else(|| format!("Plugin not found: {}", plugin_id))?;

    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    let slot_id = uuid::Uuid::new_v4().to_string();
    track.inserts.push(hardwave_project::track::PluginSlot {
        id: slot_id.clone(),
        plugin_id: descriptor.id.clone(),
        enabled: true,
        state: None,
        sidechain_source: None,
        wet: 1.0,
    });

    Ok(slot_id)
}

#[tauri::command]
pub fn remove_plugin_from_track(
    state: State<AppState>,
    track_id: String,
    slot_id: String,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();

    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    track.inserts.retain(|s| s.id != slot_id);
    drop(project);
    engine.rebuild_graph();
    Ok(())
}

#[tauri::command]
pub fn set_insert_enabled(
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    enabled: bool,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();

    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;
    let slot = track
        .inserts
        .iter_mut()
        .find(|s| s.id == slot_id)
        .ok_or_else(|| format!("Insert not found: {}", slot_id))?;
    slot.enabled = enabled;
    drop(project);
    engine.rebuild_graph();
    Ok(())
}

#[tauri::command]
pub fn reorder_insert(
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    new_index: usize,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();

    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;
    let from = track
        .inserts
        .iter()
        .position(|s| s.id == slot_id)
        .ok_or_else(|| format!("Insert not found: {}", slot_id))?;
    let to = new_index.min(track.inserts.len().saturating_sub(1));
    if from != to {
        let slot = track.inserts.remove(from);
        track.inserts.insert(to, slot);
    }
    drop(project);
    engine.rebuild_graph();
    Ok(())
}

#[tauri::command]
pub fn set_insert_wet(
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    wet: f32,
) -> Result<(), String> {
    let wet = wet.clamp(0.0, 1.0);
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();

    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;
    let slot = track
        .inserts
        .iter_mut()
        .find(|s| s.id == slot_id)
        .ok_or_else(|| format!("Insert not found: {}", slot_id))?;
    slot.wet = wet;
    drop(project);
    engine.rebuild_graph();
    Ok(())
}

#[tauri::command]
pub fn set_plugin_sidechain_source(
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    source_track_id: Option<String>,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();

    if let Some(ref src) = source_track_id {
        if src == &track_id {
            return Err("Cannot route a track's sidechain to itself".into());
        }
        if project.track(src).is_none() {
            return Err(format!("Source track not found: {}", src));
        }
    }

    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;
    let slot = track
        .inserts
        .iter_mut()
        .find(|s| s.id == slot_id)
        .ok_or_else(|| format!("Insert not found: {}", slot_id))?;
    slot.sidechain_source = source_track_id;
    drop(project);
    engine.rebuild_graph();
    Ok(())
}

#[tauri::command]
pub fn set_fx_chain_bypassed(
    state: State<AppState>,
    track_id: String,
    bypassed: bool,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();

    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;
    for slot in &mut track.inserts {
        slot.enabled = !bypassed;
    }
    drop(project);
    engine.rebuild_graph();
    Ok(())
}
