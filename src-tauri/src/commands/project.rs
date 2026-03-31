use tauri::State;
use crate::AppState;
use hardwave_project::Project;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct ProjectInfo {
    name: String,
    author: String,
    sample_rate: u32,
    track_count: usize,
    bpm: f64,
}

#[tauri::command]
pub fn new_project(state: State<AppState>) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    *project = Project::default();
}

#[tauri::command]
pub fn save_project(state: State<AppState>, path: String) -> Result<(), String> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project.save(&PathBuf::from(path)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn load_project(state: State<AppState>, path: String) -> Result<(), String> {
    let loaded = Project::load(&PathBuf::from(path)).map_err(|e| e.to_string())?;
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    *project = loaded;
    Ok(())
}

#[tauri::command]
pub fn get_project_info(state: State<AppState>) -> ProjectInfo {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    ProjectInfo {
        name: project.metadata.name.clone(),
        author: project.metadata.author.clone(),
        sample_rate: project.metadata.sample_rate,
        track_count: project.tracks.len(),
        bpm: project.tempo_map.entries[0].bpm,
    }
}
