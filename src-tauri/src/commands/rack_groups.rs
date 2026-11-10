//! The channel rack's filter groups, by name: FL's own for a song from FL
//! ("Audio", "Automation", "Unsorted"), one "Unsorted" for a song made
//! here. A channel names its group by its place in this list (a track's
//! `rackGroup`).

use crate::AppState;
use tauri::State;

#[tauri::command]
pub fn get_rack_groups(state: State<AppState>) -> Vec<String> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project.rack_groups.clone()
}
