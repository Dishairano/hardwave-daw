use crate::midi_map::MidiMapping;
use crate::AppState;
use hardwave_project::tempo::{TempoEntry, TempoMap, TempoRamp};
use hardwave_project::Project;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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
    engine.mark_song_saved();
    // The old song's plug-ins go: the new song's mixer inserts and master
    // have the same ids, and kept them (an old sampler played on).
    engine.clear_live_plugins();
    engine.rebuild_graph();
    {
        let mut m = state.midi_mappings.lock();
        m.clear();
        m.save();
    }
}

/// What belongs to the song but lives outside it until it is written:
/// every plug-in's settings as it plays now, and the MIDI Learn mappings.
pub(crate) struct LiveState {
    plugin_states: Option<std::collections::HashMap<(String, String), Vec<u8>>>,
    midi_mappings: Option<String>,
}

/// Read the live state. The plug-ins are asked BEFORE the song's lock is
/// taken: `snapshot_plugin_states` waits while the audio thread harvests
/// `get_state()` from every loaded plug-in. With a 500 ms timeout the call
/// almost always returns within one or two audio blocks, and on the rare
/// miss the song keeps the settings it last had.
pub(crate) fn live_state(state: &AppState) -> LiveState {
    let plugin_states = state
        .engine
        .lock()
        .snapshot_plugin_states(std::time::Duration::from_millis(500));
    let midi_mappings = {
        let m = state.midi_mappings.lock();
        if m.mappings.is_empty() {
            None
        } else {
            serde_json::to_string(&m.mappings).ok()
        }
    };
    LiveState {
        plugin_states,
        midi_mappings,
    }
}

impl LiveState {
    #[cfg(test)]
    pub(crate) fn for_test(
        plugin_states: std::collections::HashMap<(String, String), Vec<u8>>,
        midi_mappings: Option<String>,
    ) -> Self {
        Self {
            plugin_states: Some(plugin_states),
            midi_mappings,
        }
    }

    /// Write it into `project`, and the live timeline into the active
    /// arrangement, so the file holds exactly what is on screen and playing.
    pub(crate) fn write_into(self, project: &mut Project) {
        project.capture_active_arrangement();
        project.midi_mappings = self.midi_mappings;
        // Looked up by slot id, so `hydrate_chains_from_project` can replay
        // the bytes through `plugin.set_state(...)`. The format hint is
        // "unknown": loading only matches the slot id, and the format comes
        // from the slot's descriptor when the plug-in is made.
        if let Some(map) = self.plugin_states {
            for ((_, slot_id), bytes) in map {
                // A VST3 or CLAP that fails to hand over its state answers
                // with nothing. Written over the settings it had, the next
                // open gave it nothing to load and the patch was gone.
                let had_some = project
                    .plugin_state(&slot_id)
                    .is_some_and(|saved| !saved.chunk.is_empty());
                if bytes.is_empty() && had_some {
                    continue;
                }
                project.set_plugin_state(slot_id, "unknown", bytes);
            }
        }
    }
}

#[tauri::command]
pub fn save_project(state: State<AppState>, path: String) -> Result<(), String> {
    let live = live_state(&state);
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    live.write_into(&mut project);

    let target = PathBuf::from(&path);
    let new_dir = target.parent().map(|p| p.to_path_buf());
    follow_to_folder(
        &mut project,
        engine.project_dir().as_deref(),
        new_dir.as_deref(),
    );
    let result = project.save(&target).map_err(|e| e.to_string());
    drop(project);
    if result.is_ok() {
        engine.mark_song_saved();
        // The song lives in the new folder now: samples collected later are
        // named relative to it. A save that failed leaves it where it was,
        // and its samples with it.
        engine.set_project_dir(new_dir);
    }
    result
}

/// Ready the song to be saved into `to` from the folder it lives in, `from`.
///
/// Samples collected into the song's folder are named relative to it. Save
/// As into another folder kept those names and moved the song's base to
/// the new folder, where no such files are: the copy opened with every
/// collected sample missing, and so did the open song the next time. They
/// now become full paths into the old folder, where they really are.
fn follow_to_folder(project: &mut Project, from: Option<&Path>, to: Option<&Path>) {
    let Some(from) = from else {
        return;
    };
    let same = to.is_some_and(|to| {
        to == from
            || matches!(
                (std::fs::canonicalize(to), std::fs::canonicalize(from)),
                (Ok(a), Ok(b)) if a == b
            )
    });
    if !same {
        project.anchor_sample_files(from);
    }
}

/// Whether the song changed since it was last saved or opened, by any
/// edit that can be undone. The window asks before it closes, opens
/// another song or auto-saves, so an edit it did not see itself still
/// counts.
#[tauri::command]
pub fn song_has_unsaved_edits(state: State<AppState>) -> bool {
    state.engine.lock().song_has_unsaved_edits()
}

/// The file "Save new version" writes the song at `path` to: the next
/// version number not already in its folder.
#[tauri::command]
pub fn next_song_version_path(path: String) -> Result<String, String> {
    Project::next_version_path(std::path::Path::new(&path))
        .map(|p| p.to_string_lossy().into_owned())
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
pub(crate) struct LoadProgress {
    pub(crate) stage: &'static str,
    pub(crate) done: usize,
    pub(crate) total: usize,
}

fn load_project_blocking(app: &AppHandle, path: String) -> Result<(), String> {
    let project_file = PathBuf::from(&path);
    let loaded = Project::load(&project_file).map_err(|e| e.to_string())?;
    let project_dir = project_file.parent().map(|p| p.to_path_buf());
    open_project_blocking(app, loaded, project_dir, |_, _| {})
}

/// Make `loaded` the open song: its audio read without the engine, the
/// song swapped in under a short lock, its plug-ins created on the main
/// thread. `after_audio` gets the song once its samples are in the pool
/// and before it is swapped in (an FL import sets where its clips start
/// in their files there, which needs each file's length and rate).
pub(crate) fn open_project_blocking(
    app: &AppHandle,
    mut loaded: Project,
    project_dir: Option<PathBuf>,
    after_audio: impl FnOnce(&mut Project, &hardwave_engine::audio_pool::AudioPool),
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if OPENING.swap(true, Ordering::SeqCst) {
        return Err("A song is already opening.".into());
    }
    let _done = OpeningDone;
    let state = app.state::<AppState>();

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
    let pool = state.engine.lock().audio_pool.clone();
    after_audio(&mut loaded, &pool);

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
        // The old song's plug-ins go before the new song's are created
        // below: tracks with the same id in both songs (the mixer inserts,
        // the master) kept them, and an old sampler played on in the new
        // song. Same queue as the new plug-ins, so the order holds.
        engine.clear_live_plugins();
        engine.transport.bpm.store(new_bpm, Ordering::Relaxed);
        engine.send_command(hardwave_engine::TransportCommand::SetBpm(new_bpm));
        // Without this a project written in 7/8 opened in 4/4 until playback
        // started, because only the tempo was taken from the loaded map.
        apply_project_time_signature(&engine);
        engine.reset_history();
        engine.mark_song_saved();
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
    // The project's plug-ins not tried yet are tried here, a few at a time,
    // so their loads on the main thread below do not wait for each probe.
    {
        let state = app.state::<AppState>();
        let descriptors = crate::commands::plugins::project_descriptors(&state);
        crate::commands::plugins::probe_ahead(&state, &descriptors);
    }
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
pub fn set_channel_rack_state(
    app: AppHandle,
    state: State<AppState>,
    payload: Option<String>,
    from: Option<String>,
    undoable: Option<bool>,
) -> bool {
    let recorded = {
        let engine = state.engine.lock();
        // A step edit is an undo step like any other edit. The rack also
        // sends its state when a song is opened or another window changed
        // it; those are not edits, and neither is a payload that changes
        // nothing.
        let changed = engine.project.lock().channel_rack_state != payload;
        let recorded = changed && undoable.unwrap_or(false);
        if recorded {
            engine.snapshot_before_mutation();
        }
        // Read into the step sequencer's notes and played: the rack sends
        // this whenever a step changes, so pattern mode hears it at once.
        engine
            .project
            .lock()
            .set_channel_rack_state(payload.clone());
        engine.rebuild_graph();
        recorded
    };
    // Every other window takes the same steps, so none saves old ones.
    let _ = app.emit(
        "daw:rackChanged",
        serde_json::json!({ "payload": payload, "from": from.unwrap_or_default() }),
    );
    recorded
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

/// Everything a change to the tempo map takes with it.
///
/// Audio that keeps its speed is made to cover its own audio again
/// (`Project::follow_tempo_change`). The playhead, the loop and the punch
/// range stay on the beats they were on, as the clips do. The transport
/// reads the tempo and signature at the playhead, not the song's opening
/// ones. And the graph is built again, because notes and clips are placed in
/// samples from the map: the tempo field changed the map without that, so
/// after typing a new tempo the song kept playing at the old one until
/// something else rebuilt it.
pub(crate) fn after_tempo_change(engine: &hardwave_engine::DawEngine, before: &TempoMap) {
    use std::sync::atomic::Ordering;
    let sample_rate = f64::from(engine.current_sample_rate().max(1));
    let position = engine.transport.position();
    let loop_range = (
        engine.transport.loop_start.load(Ordering::Relaxed),
        engine.transport.loop_end.load(Ordering::Relaxed),
    );
    let (punch_on, punch_in, punch_out) = engine.punch_samples();
    let to_tick = |samples: u64| before.samples_to_tick(samples, sample_rate);
    let (new_position, new_loop) = {
        let mut project = engine.project.lock();
        project.follow_tempo_change(before);
        let map = &project.tempo_map;
        // The same tick, where the new map puts it.
        let kept = |samples: u64| map.tick_to_samples(to_tick(samples), sample_rate);
        (kept(position), (kept(loop_range.0), kept(loop_range.1)))
    };
    if new_position != position {
        engine.transport.set_position(new_position);
        engine.send_command(hardwave_engine::TransportCommand::SetPosition(new_position));
    }
    if loop_range.1 > loop_range.0 && new_loop != loop_range {
        engine
            .transport
            .loop_start
            .store(new_loop.0, Ordering::Relaxed);
        engine
            .transport
            .loop_end
            .store(new_loop.1, Ordering::Relaxed);
        engine.send_command(hardwave_engine::TransportCommand::SetLoop(
            new_loop.0, new_loop.1,
        ));
    }
    if punch_out > punch_in {
        engine.set_punch_ticks(punch_on, to_tick(punch_in), to_tick(punch_out));
    }
    transport_follows_map(engine);
    engine.rebuild_graph();
}

/// The transport's tempo and signature, read from the song's tempo map where
/// the playhead is. While playing the audio thread does this every block;
/// stopped, a new map (a tempo edit, an undo that brings back the old one)
/// left the read-out and the click on the tempo from before.
pub(crate) fn transport_follows_map(engine: &hardwave_engine::DawEngine) {
    use std::sync::atomic::Ordering;
    let sample_rate = f64::from(engine.current_sample_rate().max(1));
    let position = engine.transport.position();
    let (bpm, (num, den)) = {
        let project = engine.project.lock();
        let map = &project.tempo_map;
        let tick = map.samples_to_tick(position, sample_rate);
        (map.bpm_at(tick), map.time_sig_at(tick))
    };
    engine.transport.bpm.store(bpm, Ordering::Relaxed);
    engine.send_command(hardwave_engine::TransportCommand::SetBpm(bpm));
    engine.transport.time_sig.store(
        hardwave_engine::transport::pack_time_sig(num, den),
        Ordering::Relaxed,
    );
    engine.send_command(hardwave_engine::TransportCommand::SetTimeSignature(
        num, den,
    ));
}

/// Push the project's own time signature into the transport.
///
/// The transport's signature is a pair of atomics the audio thread reads, and
/// loading a project only ever refreshed the tempo, so the signature stayed
/// at whatever the last project used.
pub(crate) fn apply_project_time_signature(engine: &hardwave_engine::DawEngine) {
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
    if !bpm.is_finite() {
        return Err("bpm must be finite".into());
    }
    let bpm = bpm.clamp(20.0, 999.0);
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    let before = {
        let mut project = engine.project.lock();
        let before = project.tempo_map.clone();
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
        before
    };
    after_tempo_change(&engine, &before);
    Ok(())
}

#[tauri::command]
pub fn remove_tempo_entry(state: State<AppState>, index: usize) -> Result<(), String> {
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    let before = {
        let mut project = engine.project.lock();
        let before = project.tempo_map.clone();
        if index == 0 {
            return Err("Cannot remove the initial tempo entry at tick 0".into());
        }
        if index >= project.tempo_map.entries.len() {
            return Err(format!("Index {index} out of range"));
        }
        project.tempo_map.entries.remove(index);
        before
    };
    after_tempo_change(&engine, &before);
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
    if !bpm.is_finite() {
        return Err("bpm must be finite".into());
    }
    let bpm = bpm.clamp(20.0, 999.0);
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    let before = {
        let mut project = engine.project.lock();
        let before = project.tempo_map.clone();
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
        before
    };
    after_tempo_change(&engine, &before);
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
/// cadence while the window has focus. Off the main thread: it waits for
/// the engine, and a busy engine held the whole window here (the freeze log
/// on the test VM under load, 21 Oct 2026).
#[tauri::command(async)]
pub fn tick_project_working_time(state: State<'_, AppState>, seconds: u64) {
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

/// Put the channel rack's active pattern on the playlist at the bar the
/// playhead is in, one clip per channel with steps. Returns how many.
#[tauri::command]
pub fn place_pattern_on_playlist(app: AppHandle, state: State<AppState>) -> usize {
    state.engine.lock().snapshot_before_mutation();
    let (placed, ids) = {
        let engine = state.engine.lock();
        let sample_rate = f64::from(engine.current_sample_rate().max(1));
        let position = engine.transport.position();
        let mut project = engine.project.lock();
        let tick = project.tempo_map.samples_to_tick(position, sample_rate);
        let (start, per_bar) = project
            .tempo_map
            .meter_segments()
            .into_iter()
            .rev()
            .find(|(s, _)| *s <= tick)
            .unwrap_or((0, 4 * hardwave_midi::PPQ));
        let bar = start + (tick - start) / per_bar.max(1) * per_bar.max(1);
        let before: Vec<(String, usize)> = project
            .tracks
            .iter()
            .map(|t| (t.id.clone(), t.clips.len()))
            .collect();
        let placed = project.place_pattern(bar);
        let ids: Vec<String> = project
            .tracks
            .iter()
            .filter(|t| {
                before
                    .iter()
                    .any(|(id, n)| *id == t.id && *n != t.clips.len())
            })
            .map(|t| t.id.clone())
            .collect();
        drop(project);
        engine.rebuild_graph();
        (placed, ids)
    };
    for id in ids {
        crate::commands::plugins::tell_windows_track_changed(&app, &id);
    }
    placed
}

#[cfg(test)]
mod save_folder_tests {
    use super::follow_to_folder;
    use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement};
    use hardwave_project::Project;

    fn song_with_collected_kick() -> Project {
        let mut song = Project::default();
        song.track_mut("insert-001")
            .unwrap()
            .clips
            .push(ClipPlacement {
                content: ClipContent::Audio(AudioClip {
                    id: "clip-1".into(),
                    name: "kick".into(),
                    source_path: "9f2c".into(),
                    source_hash: String::new(),
                    source_start: 0,
                    source_end: 48_000,
                    gain_db: 0.0,
                    fade_in_ticks: 0,
                    fade_out_ticks: 0,
                    muted: false,
                    reversed: false,
                    pitch_semitones: 0.0,
                    stretch_ratio: 1.0,
                    fade_in_curve: Default::default(),
                    fade_out_curve: Default::default(),
                    warp_markers: Vec::new(),
                    source_file: "Anthem Samples/kick.wav".into(),
                    treatment: Default::default(),
                    tuned_notes: Vec::new(),
                }),
                track_id: "insert-001".into(),
                position_ticks: 0,
                length_ticks: 3840,
                lane: 0,
                color: None,
            });
        song
    }

    fn kick_file(song: &Project) -> String {
        match &song.track("insert-001").unwrap().clips[0].content {
            ClipContent::Audio(a) => a.source_file.clone(),
            _ => panic!("an audio clip"),
        }
    }

    #[test]
    fn save_as_into_another_folder_keeps_the_collected_samples() {
        let home = std::env::temp_dir().join("Anthem");
        let usb = std::env::temp_dir().join("USB");
        let mut song = song_with_collected_kick();
        follow_to_folder(&mut song, Some(&home), Some(&usb));
        assert_eq!(
            kick_file(&song),
            home.join("Anthem Samples/kick.wav").to_string_lossy()
        );
    }

    #[test]
    fn saving_in_its_own_folder_keeps_the_song_movable() {
        let home = std::env::temp_dir().join("Anthem");
        let mut song = song_with_collected_kick();
        follow_to_folder(&mut song, Some(&home), Some(&home));
        assert_eq!(kick_file(&song), "Anthem Samples/kick.wav");
        // A song never saved has no folder its samples could be relative to.
        follow_to_folder(&mut song, None, Some(&home));
        assert_eq!(kick_file(&song), "Anthem Samples/kick.wav");
    }
}

#[cfg(test)]
mod live_state_tests {
    use super::LiveState;
    use hardwave_project::Project;

    #[test]
    fn a_plug_in_that_hands_over_nothing_keeps_the_settings_it_had() {
        let mut song = Project::default();
        song.set_plugin_state("slot-synth", "unknown", vec![1, 2, 3]);
        song.set_plugin_state("slot-eq", "unknown", vec![4]);
        let live = LiveState::for_test(
            [
                (
                    ("insert-001".to_string(), "slot-synth".to_string()),
                    Vec::new(),
                ),
                (("insert-001".to_string(), "slot-eq".to_string()), vec![5]),
                (
                    ("insert-002".to_string(), "slot-new".to_string()),
                    Vec::new(),
                ),
            ]
            .into_iter()
            .collect(),
            None,
        );
        live.write_into(&mut song);
        assert_eq!(
            song.plugin_state("slot-synth").unwrap().chunk,
            vec![1, 2, 3]
        );
        assert_eq!(song.plugin_state("slot-eq").unwrap().chunk, vec![5]);
        assert!(song.plugin_state("slot-new").unwrap().chunk.is_empty());
    }
}
