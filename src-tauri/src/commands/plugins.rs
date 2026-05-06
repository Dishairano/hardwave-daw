use crate::AppState;
use hardwave_engine::insert_chain::{InsertCommand, LiveSlot};
use hardwave_native_plugins::{NativeCompressor, NativeEq};
use hardwave_plugin_host::scanner::ScanDiff;
use hardwave_plugin_host::types::HostedPlugin;
use hardwave_plugin_host::{
    clap_instance::ClapPluginInstance, vst3::Vst3PluginInstance, PluginDescriptor, PluginFormat,
};
use raw_window_handle::HasWindowHandle;
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};

/// Factory: turn a scanner descriptor into a live `HostedPlugin`. The
/// dispatch covers three sources:
///   * Native plug-ins shipped with the DAW (path is `<native>`).
///   * VST3 plug-ins on disk, loaded via `Vst3PluginInstance`.
///   * CLAP plug-ins on disk, loaded via `ClapPluginInstance`.
///
/// Used both by the chain hydration path (`add_plugin_to_track`,
/// `load_project`) and the editor path (`open_plugin_editor`). The
/// editor path may want a *separate* instance from the chain so the
/// returned Box is intentionally not tied to chain lifecycle.
fn instantiate_plugin(descriptor: &PluginDescriptor) -> Result<Box<dyn HostedPlugin>, String> {
    let native_path = PathBuf::from("<native>");
    if descriptor.path == native_path {
        return match descriptor.id.as_str() {
            id if id == NativeEq::ID => Ok(Box::new(NativeEq::new())),
            id if id == NativeCompressor::ID => Ok(Box::new(NativeCompressor::new())),
            other => Err(format!("Unknown native plug-in id: {other}")),
        };
    }
    match descriptor.format {
        PluginFormat::Vst3 => Ok(Box::new(
            Vst3PluginInstance::load(descriptor.clone()).map_err(|e| e.to_string())?,
        )),
        PluginFormat::Clap => Ok(Box::new(
            ClapPluginInstance::load(descriptor.clone()).map_err(|e| e.to_string())?,
        )),
    }
}

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

#[derive(Serialize)]
pub struct MissingPluginInfo {
    #[serde(rename = "pluginId")]
    pub plugin_id: String,
    #[serde(rename = "trackId")]
    pub track_id: String,
    #[serde(rename = "trackName")]
    pub track_name: String,
    #[serde(rename = "slotId")]
    pub slot_id: String,
    #[serde(rename = "slotIndex")]
    pub slot_index: usize,
}

#[tauri::command]
pub fn find_missing_plugins(state: State<AppState>) -> Vec<MissingPluginInfo> {
    let engine = state.engine.lock();
    let available: HashSet<String> = engine
        .plugin_scanner
        .lock()
        .plugins()
        .iter()
        .map(|p| p.id.clone())
        .collect();
    let project = engine.project.lock();
    let mut missing = Vec::new();
    for track in &project.tracks {
        for (slot_index, slot) in track.inserts.iter().enumerate() {
            if !available.contains(&slot.plugin_id) {
                missing.push(MissingPluginInfo {
                    plugin_id: slot.plugin_id.clone(),
                    track_id: track.id.clone(),
                    track_name: track.name.clone(),
                    slot_id: slot.id.clone(),
                    slot_index,
                });
            }
        }
    }
    missing
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

/// Open a plugin editor in a floating native window parented to the
/// main webview. Creates a fresh `HostedPlugin` instance from the
/// scanner descriptor, spawns a child Tauri window, gets its native
/// raw-window handle, and calls `open_editor(handle)` to attach the
/// plugin's `IPlugView` / CLAP GUI view there.
///
/// The instance is stored in `state.plugin_editors` keyed by the
/// Tauri window label so `close_plugin_editor` can tear it down
/// cleanly. If `open_editor` returns `false` (plugin declined or
/// platform type unsupported), the Tauri window is closed and an
/// error is returned.
#[tauri::command]
pub fn open_plugin_editor(
    app: AppHandle,
    state: State<'_, AppState>,
    plugin_id: String,
    window_label: String,
) -> Result<String, String> {
    let engine = state.engine.lock();
    let scanner = engine.plugin_scanner.lock();
    let descriptor = scanner
        .find(&plugin_id)
        .ok_or_else(|| format!("Plugin not found: {}", plugin_id))?
        .clone();
    drop(scanner);
    drop(engine);

    let mut hosted: Box<dyn HostedPlugin> = match descriptor.format {
        PluginFormat::Vst3 => {
            Box::new(Vst3PluginInstance::load(descriptor.clone()).map_err(|e| e.to_string())?)
        }
        PluginFormat::Clap => {
            Box::new(ClapPluginInstance::load(descriptor.clone()).map_err(|e| e.to_string())?)
        }
    };

    let url = tauri::WebviewUrl::App("about:blank".into());
    let editor_window = tauri::WebviewWindowBuilder::new(&app, &window_label, url)
        .title(format!("{} — Plugin Editor", descriptor.name))
        .inner_size(600.0, 400.0)
        .resizable(true)
        .always_on_top(true)
        .build()
        .map_err(|e| format!("Failed to open editor window: {e}"))?;

    let handle = editor_window
        .window_handle()
        .map_err(|e| format!("window handle unavailable: {e}"))?;
    let raw = handle.as_raw();

    if !hosted.open_editor(raw) {
        let _ = editor_window.close();
        return Err(format!(
            "{} rejected the floating-window handle (platform type unsupported or plugin has no editor)",
            descriptor.name
        ));
    }

    state
        .plugin_editors
        .lock()
        .insert(window_label.clone(), hosted);
    Ok(window_label)
}

#[tauri::command]
pub fn close_plugin_editor(
    app: AppHandle,
    state: State<'_, AppState>,
    window_label: String,
) -> Result<(), String> {
    let mut editors = state.plugin_editors.lock();
    if let Some(mut hosted) = editors.remove(&window_label) {
        hosted.close_editor();
    }
    if let Some(window) = app.get_webview_window(&window_label) {
        let _ = window.close();
    }
    Ok(())
}

#[tauri::command]
pub fn add_plugin_to_track(
    state: State<AppState>,
    track_id: String,
    plugin_id: String,
) -> Result<String, String> {
    state.engine.lock().snapshot_before_mutation();

    // Phase 1: clone descriptor and push project metadata while holding
    // the engine + project locks. Drop them before instantiation so the
    // (potentially slow) VST3 / CLAP load doesn't block other commands.
    let (descriptor, slot_id) = {
        let engine = state.engine.lock();
        let scanner = engine.plugin_scanner.lock();
        let descriptor = scanner
            .find(&plugin_id)
            .ok_or_else(|| format!("Plugin not found: {}", plugin_id))?
            .clone();
        drop(scanner);

        let mut project = engine.project.lock();
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
        (descriptor, slot_id)
    };

    // Phase 2: instantiate the plug-in off the audio path, without
    // holding any locks. VST3 / CLAP loaders may scan the bundle, dlopen
    // the library, or call into platform code — none of that is fast
    // enough to do under a Mutex.
    let plugin = instantiate_plugin(&descriptor)?;

    // Phase 3: ship the freshly-built LiveSlot to the audio thread via
    // the lock-free InsertCommand queue. The chain takes ownership and
    // calls activate() before processing the next block.
    let cmd = InsertCommand::Add {
        track_id: track_id.clone(),
        slot: LiveSlot {
            slot_id: slot_id.clone(),
            plugin,
            enabled: true,
            wet: 1.0,
        },
    };
    state
        .engine
        .lock()
        .try_send_insert_command(cmd)
        .map_err(|_| "insert command queue full or engine not started".to_string())?;

    // Drain graveyard opportunistically so prior removes don't pile up
    // before something else triggers a drain.
    state.engine.lock().drain_insert_graveyard();

    Ok(slot_id)
}

#[tauri::command]
pub fn remove_plugin_from_track(
    state: State<AppState>,
    track_id: String,
    slot_id: String,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {}", track_id))?;
        track.inserts.retain(|s| s.id != slot_id);
    }
    let cmd = InsertCommand::Remove {
        track_id: track_id.clone(),
        slot_id: slot_id.clone(),
    };
    let _ = state.engine.lock().try_send_insert_command(cmd);
    state.engine.lock().drain_insert_graveyard();
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
    {
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
    }
    let cmd = InsertCommand::SetEnabled {
        track_id: track_id.clone(),
        slot_id: slot_id.clone(),
        enabled,
    };
    let _ = state.engine.lock().try_send_insert_command(cmd);
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
    let (from, to) = {
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
        (from, to)
    };
    if from != to {
        let cmd = InsertCommand::Reorder {
            track_id: track_id.clone(),
            from,
            to,
        };
        let _ = state.engine.lock().try_send_insert_command(cmd);
    }
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
    {
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
    }
    let cmd = InsertCommand::SetWet {
        track_id: track_id.clone(),
        slot_id: slot_id.clone(),
        wet,
    };
    let _ = state.engine.lock().try_send_insert_command(cmd);
    Ok(())
}

/// Live parameter change for a chain-resident plug-in. Sends a
/// `SetParameter` command to the audio thread so the next block
/// reflects the new value. Project state is NOT updated here — the
/// caller is responsible for snapshotting periodically (or the editor
/// can store a parameter map separately for save/load).
#[tauri::command]
pub fn set_plugin_parameter(
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    param_id: u32,
    value: f64,
) -> Result<(), String> {
    let cmd = InsertCommand::SetParameter {
        track_id,
        slot_id,
        param_id,
        value,
    };
    state
        .engine
        .lock()
        .try_send_insert_command(cmd)
        .map_err(|_| "insert command queue full or engine not started".to_string())?;
    Ok(())
}

/// Hydrate every persisted PluginSlot in the current project into the
/// audio thread's chains. Called from `load_project` after the project
/// state has been replaced. For each insert: instantiate the plug-in,
/// ship an Add command. Plug-ins missing from the scanner cache are
/// skipped with a warning so the project still opens — the user gets a
/// "missing plug-ins" notice via `find_missing_plugins`.
pub fn hydrate_chains_from_project(state: &AppState) -> Result<(), String> {
    // Snapshot what we need under the locks, then drop them before we
    // start instantiating plug-ins (slow VST3 / CLAP loads).
    let plan: Vec<(String, String, PluginDescriptor, bool, f32)> = {
        let engine = state.engine.lock();
        let project = engine.project.lock();
        let scanner = engine.plugin_scanner.lock();
        let mut acc = Vec::new();
        for track in &project.tracks {
            for slot in &track.inserts {
                if let Some(descriptor) = scanner.find(&slot.plugin_id) {
                    acc.push((
                        track.id.clone(),
                        slot.id.clone(),
                        descriptor.clone(),
                        slot.enabled,
                        slot.wet,
                    ));
                } else {
                    log::warn!(
                        "load_project: skipping missing plug-in {} on track {}",
                        slot.plugin_id,
                        track.id
                    );
                }
            }
        }
        acc
    };

    for (track_id, slot_id, descriptor, enabled, wet) in plan {
        match instantiate_plugin(&descriptor) {
            Ok(plugin) => {
                let cmd = InsertCommand::Add {
                    track_id,
                    slot: LiveSlot {
                        slot_id,
                        plugin,
                        enabled,
                        wet,
                    },
                };
                if state.engine.lock().try_send_insert_command(cmd).is_err() {
                    log::warn!("hydrate: insert command queue full, will retry on next save");
                    break;
                }
            }
            Err(e) => log::warn!("hydrate: failed to load {}: {e}", descriptor.id),
        }
    }
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
