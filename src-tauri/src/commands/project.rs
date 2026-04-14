use crate::AppState;
use hardwave_project::Project;
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

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
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    let new_bpm = {
        let mut project = engine.project.lock();
        *project = Project::default();
        project
            .tempo_map
            .entries
            .first()
            .map(|e| e.bpm)
            .unwrap_or(140.0)
    };
    engine.transport.bpm.store(new_bpm, Ordering::Relaxed);
    engine.send_command(hardwave_engine::TransportCommand::SetBpm(new_bpm));
    engine.reset_history();
    engine.rebuild_graph();
}

#[tauri::command]
pub fn save_project(state: State<AppState>, path: String) -> Result<(), String> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project
        .save(&PathBuf::from(path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn load_project(state: State<AppState>, path: String) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let loaded = Project::load(&PathBuf::from(path)).map_err(|e| e.to_string())?;
    let engine = state.engine.lock();
    let new_bpm = loaded
        .tempo_map
        .entries
        .first()
        .map(|e| e.bpm)
        .unwrap_or(140.0);
    {
        let mut project = engine.project.lock();
        *project = loaded;
    }
    engine.transport.bpm.store(new_bpm, Ordering::Relaxed);
    engine.send_command(hardwave_engine::TransportCommand::SetBpm(new_bpm));
    engine.reset_history();
    engine.rebuild_graph();
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
