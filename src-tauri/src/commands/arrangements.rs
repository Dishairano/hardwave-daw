//! Arrangement commands — switchable playlists within one project.
//! Switching captures the live timeline into the active arrangement, then
//! applies the target's snapshot onto the tracks and rebuilds the graph.
//! Model + logic live in `hardwave_project::arrangement`.

use crate::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct ArrangementInfo {
    pub id: String,
    pub name: String,
    pub active: bool,
}

#[tauri::command]
pub fn list_arrangements(state: State<AppState>) -> Vec<ArrangementInfo> {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    project.ensure_arrangements();
    let active = project.active_arrangement.clone();
    project
        .arrangements
        .iter()
        .map(|a| ArrangementInfo {
            id: a.id.clone(),
            name: a.name.clone(),
            active: a.id == active,
        })
        .collect()
}

/// Create a new arrangement and switch to it. `copy_current` clones the
/// live timeline into it (duplicate); otherwise it starts empty.
#[tauri::command]
pub fn create_arrangement(
    state: State<AppState>,
    name: String,
    copy_current: bool,
) -> Result<String, String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let id = {
        let mut project = engine.project.lock();
        project.create_arrangement(&name, copy_current)
    };
    engine.rebuild_graph();
    Ok(id)
}

#[tauri::command]
pub fn switch_arrangement(state: State<AppState>, id: String) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        project.capture_active_arrangement();
        project.apply_arrangement(&id)?;
    }
    engine.rebuild_graph();
    Ok(())
}

#[tauri::command]
pub fn rename_arrangement(state: State<AppState>, id: String, name: String) -> Result<(), String> {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    project.rename_arrangement(&id, &name)
}

#[tauri::command]
pub fn delete_arrangement(state: State<AppState>, id: String) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        project.delete_arrangement(&id)?;
    }
    engine.rebuild_graph();
    Ok(())
}
