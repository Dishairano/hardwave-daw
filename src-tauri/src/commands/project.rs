use crate::midi_map::MidiMapping;
use crate::AppState;
use hardwave_project::tempo::{TempoEntry, TempoRamp};
use hardwave_project::Project;
use serde::{Deserialize, Serialize};
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
    {
        let mut m = state.midi_mappings.lock();
        m.clear();
        m.save();
    }
}

#[tauri::command]
pub fn save_project(state: State<AppState>, path: String) -> Result<(), String> {
    // Ask the audio thread to snapshot plug-in state BEFORE we take the
    // project lock. snapshot_plugin_states blocks the UI thread on a
    // SyncReceiver while the audio thread harvests `get_state()` from
    // every loaded plug-in. With a 500 ms timeout the call almost
    // always returns within one or two audio blocks — and on the rare
    // miss we silently fall back to whatever was previously written.
    let snapshot = {
        let engine = state.engine.lock();
        engine.snapshot_plugin_states(std::time::Duration::from_millis(500))
    };

    let mapping_blob = {
        let m = state.midi_mappings.lock();
        if m.mappings.is_empty() {
            None
        } else {
            serde_json::to_string(&m.mappings).ok()
        }
    };
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    project.midi_mappings = mapping_blob;

    // Write the harvested plug-in states into the project before we
    // serialize. Looked up by slot_id so future `hydrate_chains_from_project`
    // can replay the bytes via `plugin.set_state(...)`.
    if let Some(map) = snapshot {
        for (track_id, slot_id) in map.keys().cloned().collect::<Vec<_>>() {
            // Format hint defaults to "unknown" — load only cares about
            // matching slot_id, the format is preserved per slot's
            // descriptor lookup at hydrate time.
            let bytes = map[&(track_id.clone(), slot_id.clone())].clone();
            project.set_plugin_state(slot_id, "unknown", bytes);
        }
    }

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
    let mapping_blob = loaded.midi_mappings.clone();
    {
        let mut project = engine.project.lock();
        *project = loaded;
    }
    engine.transport.bpm.store(new_bpm, Ordering::Relaxed);
    engine.send_command(hardwave_engine::TransportCommand::SetBpm(new_bpm));
    engine.reset_history();
    engine.rebuild_graph();
    drop(engine);

    // Rehydrate plug-in chains for every PluginSlot in the loaded
    // project. Done after rebuild_graph so the freshly-built TrackNodes
    // are reachable by the dispatcher; failures (missing plug-in,
    // queue full) are logged and the project still opens — the user
    // surfaces them via `find_missing_plugins`.
    if let Err(e) = crate::commands::plugins::hydrate_chains_from_project(&state) {
        log::warn!("load_project: chain hydration failed: {e}");
    }

    {
        let mut m = state.midi_mappings.lock();
        match mapping_blob.as_deref() {
            Some(blob) => match serde_json::from_str::<Vec<MidiMapping>>(blob) {
                Ok(parsed) => {
                    m.mappings = parsed;
                }
                Err(e) => {
                    log::warn!("load_project: midi_mappings parse failed: {e}");
                    m.clear();
                }
            },
            None => m.clear(),
        }
        m.save();
    }
    Ok(())
}

#[tauri::command]
pub fn get_channel_rack_state(state: State<AppState>) -> Option<String> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project.channel_rack_state.clone()
}

#[tauri::command]
pub fn set_channel_rack_state(state: State<AppState>, payload: Option<String>) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    project.channel_rack_state = payload;
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TempoEntryInfo {
    pub tick: u64,
    pub bpm: f64,
    #[serde(rename = "timeSigNum")]
    pub time_sig_num: u32,
    #[serde(rename = "timeSigDen")]
    pub time_sig_den: u32,
    pub ramp: String,
}

fn ramp_to_str(r: TempoRamp) -> String {
    match r {
        TempoRamp::Instant => "instant".to_string(),
        TempoRamp::Linear => "linear".to_string(),
    }
}

fn ramp_from_str(s: &str) -> TempoRamp {
    match s {
        "linear" => TempoRamp::Linear,
        _ => TempoRamp::Instant,
    }
}

#[tauri::command]
pub fn get_tempo_entries(state: State<AppState>) -> Vec<TempoEntryInfo> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project
        .tempo_map
        .entries
        .iter()
        .map(|e| TempoEntryInfo {
            tick: e.tick,
            bpm: e.bpm,
            time_sig_num: e.time_sig_num,
            time_sig_den: e.time_sig_den,
            ramp: ramp_to_str(e.ramp),
        })
        .collect()
}

#[tauri::command]
pub fn add_tempo_entry(
    state: State<AppState>,
    tick: u64,
    bpm: f64,
    ramp: String,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if !bpm.is_finite() {
        return Err("bpm must be finite".into());
    }
    let bpm = bpm.clamp(20.0, 999.0);
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    {
        let mut project = engine.project.lock();
        if tick == 0 {
            return Err("Cannot add entry at tick 0 (that is the initial entry). Edit the first entry instead.".into());
        }
        if project.tempo_map.entries.iter().any(|e| e.tick == tick) {
            return Err(format!("Tempo entry already exists at tick {tick}"));
        }
        let (num, den) = project
            .tempo_map
            .entries
            .iter()
            .rev()
            .find(|e| e.tick < tick)
            .map(|e| (e.time_sig_num, e.time_sig_den))
            .unwrap_or((4, 4));
        project.tempo_map.entries.push(TempoEntry {
            tick,
            bpm,
            time_sig_num: num,
            time_sig_den: den,
            ramp: ramp_from_str(&ramp),
        });
        project.tempo_map.entries.sort_by_key(|e| e.tick);
        let first_bpm = project.tempo_map.entries[0].bpm;
        engine.transport.bpm.store(first_bpm, Ordering::Relaxed);
    }
    engine.send_command(hardwave_engine::TransportCommand::SetBpm(
        engine.transport.bpm.load(Ordering::Relaxed),
    ));
    engine.rebuild_graph();
    Ok(())
}

#[tauri::command]
pub fn remove_tempo_entry(state: State<AppState>, index: usize) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    {
        let mut project = engine.project.lock();
        if index == 0 {
            return Err("Cannot remove the initial tempo entry at tick 0".into());
        }
        if index >= project.tempo_map.entries.len() {
            return Err(format!("Index {index} out of range"));
        }
        project.tempo_map.entries.remove(index);
        let first_bpm = project.tempo_map.entries[0].bpm;
        engine.transport.bpm.store(first_bpm, Ordering::Relaxed);
    }
    engine.send_command(hardwave_engine::TransportCommand::SetBpm(
        engine.transport.bpm.load(Ordering::Relaxed),
    ));
    engine.rebuild_graph();
    Ok(())
}

#[tauri::command]
pub fn set_tempo_entry(
    state: State<AppState>,
    index: usize,
    tick: u64,
    bpm: f64,
    ramp: String,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if !bpm.is_finite() {
        return Err("bpm must be finite".into());
    }
    let bpm = bpm.clamp(20.0, 999.0);
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    {
        let mut project = engine.project.lock();
        if index >= project.tempo_map.entries.len() {
            return Err(format!("Index {index} out of range"));
        }
        let new_tick = if index == 0 { 0 } else { tick };
        if project
            .tempo_map
            .entries
            .iter()
            .enumerate()
            .any(|(i, e)| i != index && e.tick == new_tick)
        {
            return Err(format!("Tempo entry already exists at tick {new_tick}"));
        }
        {
            let entry = &mut project.tempo_map.entries[index];
            entry.tick = new_tick;
            entry.bpm = bpm;
            entry.ramp = ramp_from_str(&ramp);
        }
        project.tempo_map.entries.sort_by_key(|e| e.tick);
        let first_bpm = project.tempo_map.entries[0].bpm;
        engine.transport.bpm.store(first_bpm, Ordering::Relaxed);
    }
    engine.send_command(hardwave_engine::TransportCommand::SetBpm(
        engine.transport.bpm.load(Ordering::Relaxed),
    ));
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

/// Full Project Info dialog payload. Mirrors FL Studio's Project Info
/// fields one-to-one: title / genre / author / info / url + the
/// "Show on open" splash toggle + the cumulative working-time counter.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ProjectInfoMeta {
    pub title: String,
    pub author: String,
    pub genre: String,
    pub info: String,
    pub url: String,
    pub show_on_open: bool,
    pub working_time_seconds: u64,
}

#[tauri::command]
pub fn get_project_meta(state: State<AppState>) -> ProjectInfoMeta {
    let engine = state.engine.lock();
    let m = &engine.project.lock().metadata;
    ProjectInfoMeta {
        title: m.title.clone(),
        author: m.author.clone(),
        genre: m.genre.clone(),
        info: m.info.clone(),
        url: m.url.clone(),
        show_on_open: m.show_on_open,
        working_time_seconds: m.working_time_seconds,
    }
}

#[tauri::command]
pub fn set_project_meta(state: State<AppState>, meta: ProjectInfoMeta) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    project.metadata.title = meta.title;
    project.metadata.author = meta.author;
    project.metadata.genre = meta.genre;
    project.metadata.info = meta.info;
    project.metadata.url = meta.url;
    project.metadata.show_on_open = meta.show_on_open;
    project.metadata.working_time_seconds = meta.working_time_seconds;
    project.metadata.modified_at = chrono::Utc::now().to_rfc3339();
}

/// Reset the cumulative working-time counter to zero. Wired to the
/// "Reset working time" button on the Project Info dialog so users can
/// kick off a fresh session counter without touching anything else.
#[tauri::command]
pub fn reset_project_working_time(state: State<AppState>) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    project.metadata.working_time_seconds = 0;
    project.metadata.modified_at = chrono::Utc::now().to_rfc3339();
}

/// Increment the working-time counter. The UI calls this on a 30s
/// cadence while the window has focus.
#[tauri::command]
pub fn tick_project_working_time(state: State<AppState>, seconds: u64) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    project.metadata.working_time_seconds = project
        .metadata
        .working_time_seconds
        .saturating_add(seconds);
}
