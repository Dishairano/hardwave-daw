//! VCA groups: one fader that rides several others.
//!
//! The model and the arithmetic live in `hardwave_project::vca`. This file
//! is the part the app calls: every change rebuilds the graph, because the
//! group's level is folded into each member's fader there.

use crate::AppState;
use hardwave_project::vca::Vca;
use tauri::State;

#[tauri::command]
pub fn list_vcas(state: State<AppState>) -> Vec<Vca> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project.vcas.clone()
}

#[tauri::command]
pub fn add_vca(state: State<AppState>, name: String) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("give the group a name".into());
    }
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let id = uuid::Uuid::new_v4().to_string();
    project.vcas.push(Vca::new(id.clone(), name));
    Ok(id)
}

#[tauri::command]
pub fn rename_vca(state: State<AppState>, id: String, name: String) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("give the group a name".into());
    }
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let vca = project
        .vcas
        .iter_mut()
        .find(|v| v.id == id)
        .ok_or_else(|| "no group with that id".to_string())?;
    vca.name = name;
    Ok(())
}

/// Remove a group. The members keep their own faders exactly as they are,
/// so the mix changes by whatever the group was adding.
#[tauri::command]
pub fn delete_vca(state: State<AppState>, id: String) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let before = project.vcas.len();
        project.vcas.retain(|v| v.id != id);
        if project.vcas.len() == before {
            return Err("no group with that id".into());
        }
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Move the group's fader.
///
/// No undo snapshot: this is a fader, and a whole move would fill the
/// history. The value is saved with the song.
#[tauri::command]
pub fn set_vca_gain(state: State<AppState>, id: String, gain_db: f64) -> Result<(), String> {
    if !gain_db.is_finite() {
        return Err("that is not a level".into());
    }
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let vca = project
            .vcas
            .iter_mut()
            .find(|v| v.id == id)
            .ok_or_else(|| "no group with that id".to_string())?;
        vca.gain_db = gain_db.clamp(-60.0, 12.0);
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

#[tauri::command]
pub fn set_vca_muted(state: State<AppState>, id: String, muted: bool) -> Result<(), String> {
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let vca = project
            .vcas
            .iter_mut()
            .find(|v| v.id == id)
            .ok_or_else(|| "no group with that id".to_string())?;
        vca.muted = muted;
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Say which tracks the group rides.
///
/// A track can be in more than one group, and a track that no longer
/// exists is dropped rather than kept as a name that means nothing.
#[tauri::command]
pub fn set_vca_members(
    state: State<AppState>,
    id: String,
    members: Vec<String>,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let live: Vec<String> = members
            .into_iter()
            .filter(|m| project.tracks.iter().any(|t| &t.id == m))
            .collect();
        let vca = project
            .vcas
            .iter_mut()
            .find(|v| v.id == id)
            .ok_or_else(|| "no group with that id".to_string())?;
        vca.members = live;
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}
