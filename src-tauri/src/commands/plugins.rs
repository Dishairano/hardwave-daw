use tauri::State;
use crate::AppState;
use hardwave_plugin_host::PluginDescriptor;

#[tauri::command]
pub fn scan_plugins(state: State<AppState>) -> Vec<PluginDescriptor> {
    let engine = state.engine.lock();
    let mut scanner = engine.plugin_scanner.lock();
    scanner.scan().to_vec()
}

#[tauri::command]
pub fn get_plugins(state: State<AppState>) -> Vec<PluginDescriptor> {
    let engine = state.engine.lock();
    let scanner = engine.plugin_scanner.lock();
    scanner.plugins().to_vec()
}

#[tauri::command]
pub fn add_plugin_to_track(
    state: State<AppState>,
    track_id: String,
    plugin_id: String,
) -> Result<String, String> {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let scanner = engine.plugin_scanner.lock();

    let descriptor = scanner.find(&plugin_id)
        .ok_or_else(|| format!("Plugin not found: {}", plugin_id))?;

    let track = project.track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    let slot_id = uuid::Uuid::new_v4().to_string();
    track.inserts.push(hardwave_project::track::PluginSlot {
        id: slot_id.clone(),
        plugin_id: descriptor.id.clone(),
        enabled: true,
        state: None,
        sidechain_source: None,
    });

    Ok(slot_id)
}

#[tauri::command]
pub fn remove_plugin_from_track(
    state: State<AppState>,
    track_id: String,
    slot_id: String,
) -> Result<(), String> {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();

    let track = project.track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    track.inserts.retain(|s| s.id != slot_id);
    Ok(())
}
