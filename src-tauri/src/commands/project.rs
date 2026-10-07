use crate::midi_map::MidiMapping;
use crate::AppState;
use hardwave_project::tempo::{TempoEntry, TempoRamp};
use hardwave_project::Project;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};

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
    apply_project_time_signature(&engine);
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
    // Flush the live timeline into the active arrangement so the saved
    // file reflects exactly what's on screen.
    project.capture_active_arrangement();
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

    let target = PathBuf::from(&path);
    let result = project.save(&target).map_err(|e| e.to_string());
    drop(project);
    // Save As moves the project to a new folder, and samples collected into
    // the old one are stored relative to it, so the base has to follow.
    engine.set_project_dir(target.parent().map(|p| p.to_path_buf()));
    result
}

/// Open a song. Off the window's thread: as a sync command the whole open
/// (reading the file, decoding every sample, loading every plug-in) ran on
/// it, under the engine's lock, and the window said Not Responding until it
/// was done. The audio is now read without the lock, the song swapped in
/// under a short one, and the plug-ins created on the main thread as a
/// posted task. `project-load-progress` follows along.
#[tauri::command]
pub async fn load_project(app: AppHandle, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || load_project_blocking(&app, path))
        .await
        .map_err(|e| format!("opening stopped: {e}"))?
}

/// One song opens at a time.
static OPENING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct OpeningDone;
impl Drop for OpeningDone {
    fn drop(&mut self) {
        OPENING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LoadProgress {
    stage: &'static str,
    done: usize,
    total: usize,
}

fn load_project_blocking(app: &AppHandle, path: String) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if OPENING.swap(true, Ordering::SeqCst) {
        return Err("A song is already opening.".into());
    }
    let _done = OpeningDone;
    let state = app.state::<AppState>();

    let project_file = PathBuf::from(&path);
    let loaded = Project::load(&project_file).map_err(|e| e.to_string())?;
    let project_dir = project_file.parent().map(|p| p.to_path_buf());

    // The audio first, without the engine. A collected project stores its
    // samples relative to its folder, so the loader is given that folder.
    let loader = state.engine.lock().audio_source_loader(project_dir.clone());
    let missing_audio = loader.load_sources_of(&loaded, |done, total| {
        let _ = app.emit(
            "project-load-progress",
            LoadProgress {
                stage: "audio",
                done,
                total,
            },
        );
    });
    if !missing_audio.is_empty() {
        log::warn!(
            "load_project: {} audio source(s) missing on disk: {:?}",
            missing_audio.len(),
            missing_audio
        );
    }

    let new_bpm = loaded
        .tempo_map
        .entries
        .first()
        .map(|e| e.bpm)
        .unwrap_or(140.0);
    let mapping_blob = loaded.midi_mappings.clone();
    {
        let engine = state.engine.lock();
        engine.set_project_dir(project_dir);
        {
            let mut project = engine.project.lock();
            *project = loaded;
        }
        engine.transport.bpm.store(new_bpm, Ordering::Relaxed);
        engine.send_command(hardwave_engine::TransportCommand::SetBpm(new_bpm));
        // Without this a project written in 7/8 opened in 4/4 until playback
        // started, because only the tempo was taken from the loaded map.
        apply_project_time_signature(&engine);
        engine.reset_history();
        engine.rebuild_graph();
        // The master's fader is the master level.
        let master = engine
            .project
            .lock()
            .tracks
            .iter()
            .find(|t| matches!(t.kind, hardwave_project::TrackKind::Master))
            .map(|t| t.id.clone());
        if let Some(master) = master {
            engine.apply_track_mix(&master);
        }
    }

    // Plug-ins for every slot, created on the main thread where plug-ins
    // expect to be made, as a posted task rather than inside the window's
    // own callback. Failures (missing plug-in, queue full) are logged and
    // the song still opens; `find_missing_plugins` names them.
    let _ = app.emit(
        "project-load-progress",
        LoadProgress {
            stage: "plugins",
            done: 0,
            total: 1,
        },
    );
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let for_main = app.clone();
    app.run_on_main_thread(move || {
        let state = for_main.state::<AppState>();
        let _ = tx.send(crate::commands::plugins::hydrate_chains_from_project(
            &state,
        ));
    })
    .map_err(|e| format!("could not reach the main thread: {e}"))?;
    match rx.recv() {
        Ok(Err(e)) => log::warn!("load_project: chain hydration failed: {e}"),
        Err(_) => log::warn!("load_project: chain hydration did not report back"),
        Ok(Ok(())) => {}
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

    // The chains are up and the graph is built, so the saved macro values
    // can be pushed. Without this a song opens with its macro knobs where
    // they were left but the plug-ins at whatever the preset says.
    crate::commands::macros::apply_all_macros(state);
    let _ = app.emit(
        "project-load-progress",
        LoadProgress {
            stage: "done",
            done: 1,
            total: 1,
        },
    );
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

/// Markers and the punch range, as the UI serialises them. Opaque here on
/// purpose: the shape belongs to the frontend, this only has to survive
/// save and load with the project instead of in browser storage.
#[tauri::command]
pub fn get_timeline_state(state: State<AppState>) -> Option<String> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project.timeline_state.clone()
}

#[tauri::command]
pub fn set_timeline_state(state: State<AppState>, payload: Option<String>) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    // MERGE, do not replace. This blob started as the markers and the punch
    // range, which the UI owns, and now also carries things the backend
    // owns: mixer snapshots and grooves. The UI sends only its own keys, so
    // replacing the blob wholesale threw the rest away the next time a
    // marker moved.
    let Some(raw) = payload else {
        project.timeline_state = None;
        return;
    };
    let incoming: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        // Not JSON: keep the old behaviour rather than silently dropping it.
        Err(_) => {
            project.timeline_state = Some(raw);
            return;
        }
    };
    let mut merged: serde_json::Value = project
        .timeline_state
        .as_deref()
        .and_then(|existing| serde_json::from_str(existing).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    match (merged.as_object_mut(), incoming.as_object()) {
        (Some(target), Some(source)) => {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
            project.timeline_state = Some(merged.to_string());
        }
        // Either side is not an object: the incoming value is what the
        // caller asked for.
        _ => project.timeline_state = Some(raw),
    }
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

/// Push the project's own time signature into the transport.
///
/// The transport's signature is a pair of atomics the audio thread reads, and
/// loading a project only ever refreshed the tempo, so the signature stayed
/// at whatever the last project used.
fn apply_project_time_signature(engine: &hardwave_engine::DawEngine) {
    use std::sync::atomic::Ordering;
    let (num, den) = {
        let project = engine.project.lock();
        project.tempo_map.time_sig_at(0)
    };
    engine.transport.time_sig.store(
        hardwave_engine::transport::pack_time_sig(num, den),
        Ordering::Relaxed,
    );
    engine.send_command(hardwave_engine::TransportCommand::SetTimeSignature(
        num, den,
    ));
}

/// Change the time signature of one tempo-map entry.
///
/// This is how a signature change part-way through a song is made: the entry
/// holds it, the engine reads it at the playhead, and the playlist draws its
/// bars from it. Entries after this one that inherited the old signature are
/// left alone, so the change applies from here to the next deliberate change.
#[tauri::command]
pub fn set_tempo_entry_time_signature(
    state: State<AppState>,
    index: usize,
    numerator: u32,
    denominator: u32,
) -> Result<(), String> {
    let (num, den) = validate_time_signature(numerator, denominator)?;
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    {
        let mut project = engine.project.lock();
        if index >= project.tempo_map.entries.len() {
            return Err(format!("Index {index} out of range"));
        }
        let entry = &mut project.tempo_map.entries[index];
        entry.time_sig_num = num;
        entry.time_sig_den = den;
    }
    apply_project_time_signature(&engine);
    engine.rebuild_graph();
    Ok(())
}

/// A signature the rest of the app can count in.
///
/// The denominator has to be a power of two, because a beat is a note value:
/// 4/5 has no note length to count. The numerator is capped where a bar stops
/// being a bar anyone reads.
pub fn validate_time_signature(numerator: u32, denominator: u32) -> Result<(u32, u32), String> {
    if !(1..=64).contains(&numerator) {
        return Err(format!("Numerator {numerator} is outside 1-64"));
    }
    if !matches!(denominator, 1 | 2 | 4 | 8 | 16 | 32) {
        return Err(format!(
            "Denominator {denominator} is not a note length (1, 2, 4, 8, 16 or 32)"
        ));
    }
    Ok((numerator, denominator))
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

#[cfg(test)]
mod timeline_state_merge_tests {
    /// The merge itself, without a running engine: the UI sends its own
    /// keys, and whatever else is in the blob has to survive.
    fn merge(existing: Option<&str>, incoming: &str) -> String {
        let incoming: serde_json::Value = serde_json::from_str(incoming).unwrap();
        let mut merged: serde_json::Value = existing
            .and_then(|e| serde_json::from_str(e).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        if let (Some(target), Some(source)) = (merged.as_object_mut(), incoming.as_object()) {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
        merged.to_string()
    }

    #[test]
    fn what_the_backend_owns_survives_a_marker_move() {
        let existing =
            r#"{"markers":[],"mixerSnapshots":[{"name":"Mix A"}],"grooves":[{"name":"swing"}]}"#;
        let from_ui = r#"{"markers":[{"id":"m1","tick":960}],"punch":{"enabled":false}}"#;
        let merged = merge(Some(existing), from_ui);
        assert!(
            merged.contains("Mix A"),
            "the snapshots are still there: {merged}"
        );
        assert!(
            merged.contains("swing"),
            "the grooves are still there: {merged}"
        );
        assert!(merged.contains("m1"), "and the new marker landed");
    }

    #[test]
    fn a_first_write_needs_nothing_to_merge_into() {
        let merged = merge(None, r#"{"markers":[]}"#);
        assert_eq!(merged, r#"{"markers":[]}"#);
    }
}
