use crate::AppState;
use hardwave_engine::insert_chain::{InsertCommand, LiveSlot};
use hardwave_native_plugins::{
    NativeAutoFilter, NativeAutoPan, NativeBitcrush, NativeChorus, NativeClipper, NativeCompressor,
    NativeConvReverb, NativeDelay, NativeDistortion, NativeEq, NativeExciter, NativeFilter,
    NativeFlanger, NativeFmSynth, NativeGain, NativeGate, NativeLimiter, NativeMidSide,
    NativeMonoFold, NativeMultiband, NativeNoise, NativePhaser, NativeReverb, NativeRingMod,
    NativeSampler, NativeSaturator, NativeSoundgoodizer, NativeStereo, NativeStereoDouble,
    NativeStutter, NativeSubBass, NativeTape, NativeTransient, NativeTremolo, NativeTripleOsc,
    NativeVibrato, NativeVocoder, NativeWavetable,
};
use hardwave_plugin_host::scanner::ScanDiff;
use hardwave_plugin_host::types::HostedPlugin;
use hardwave_plugin_host::{
    clap_instance::ClapPluginInstance, vst3::Vst3PluginInstance, PluginDescriptor, PluginFormat,
};
// On Windows the plug-in area is made in plugin_window_host, which takes the handle itself.
#[cfg(not(windows))]
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
/// The descriptor a sandbox child needs, built from what the command
/// line gave it.
///
/// A child is started with a path and an id rather than a scan: it has
/// one plug-in to load and no reason to walk the disk.
pub(crate) fn descriptor_for_child(path: &str, id: &str) -> Option<PluginDescriptor> {
    let path_buf = PathBuf::from(path);
    if path != "<native>" && !path_buf.exists() {
        return None;
    }
    let format = if path.to_lowercase().ends_with(".clap") {
        hardwave_plugin_host::types::PluginFormat::Clap
    } else {
        hardwave_plugin_host::types::PluginFormat::Vst3
    };
    Some(PluginDescriptor {
        id: id.to_string(),
        name: path_buf
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(id)
            .to_string(),
        vendor: String::new(),
        version: String::new(),
        format,
        path: path_buf,
        category: hardwave_plugin_host::types::PluginCategory::Effect,
        num_inputs: 2,
        num_outputs: 2,
        has_midi_input: false,
        has_editor: false,
    })
}

pub(crate) fn instantiate_plugin(
    descriptor: &PluginDescriptor,
) -> Result<Box<dyn HostedPlugin>, String> {
    let native_path = PathBuf::from("<native>");
    if descriptor.path == native_path {
        return match descriptor.id.as_str() {
            id if id == NativeEq::ID => Ok(Box::new(NativeEq::new())),
            id if id == NativeCompressor::ID => Ok(Box::new(NativeCompressor::new())),
            id if id == NativeLimiter::ID => Ok(Box::new(NativeLimiter::new())),
            id if id == NativeDistortion::ID => Ok(Box::new(NativeDistortion::new())),
            id if id == NativeFilter::ID => Ok(Box::new(NativeFilter::new())),
            id if id == NativeDelay::ID => Ok(Box::new(NativeDelay::new())),
            id if id == NativeReverb::ID => Ok(Box::new(NativeReverb::new())),
            id if id == NativeStereo::ID => Ok(Box::new(NativeStereo::new())),
            id if id == NativeMultiband::ID => Ok(Box::new(NativeMultiband::new())),
            id if id == NativeTripleOsc::ID => Ok(Box::new(NativeTripleOsc::new())),
            id if id == NativeFmSynth::ID => Ok(Box::new(NativeFmSynth::new())),
            id if id == NativeWavetable::ID => Ok(Box::new(NativeWavetable::new())),
            id if id == NativeChorus::ID => Ok(Box::new(NativeChorus::new())),
            id if id == NativePhaser::ID => Ok(Box::new(NativePhaser::new())),
            id if id == NativeConvReverb::ID => Ok(Box::new(NativeConvReverb::new())),
            id if id == NativeTremolo::ID => Ok(Box::new(NativeTremolo::new())),
            id if id == NativeFlanger::ID => Ok(Box::new(NativeFlanger::new())),
            id if id == NativeAutoPan::ID => Ok(Box::new(NativeAutoPan::new())),
            id if id == NativeBitcrush::ID => Ok(Box::new(NativeBitcrush::new())),
            id if id == NativeGain::ID => Ok(Box::new(NativeGain::new())),
            id if id == NativeSaturator::ID => Ok(Box::new(NativeSaturator::new())),
            id if id == NativeNoise::ID => Ok(Box::new(NativeNoise::new())),
            id if id == NativeSubBass::ID => Ok(Box::new(NativeSubBass::new())),
            id if id == NativeVibrato::ID => Ok(Box::new(NativeVibrato::new())),
            id if id == NativeMidSide::ID => Ok(Box::new(NativeMidSide::new())),
            id if id == NativeGate::ID => Ok(Box::new(NativeGate::new())),
            id if id == NativeTransient::ID => Ok(Box::new(NativeTransient::new())),
            id if id == NativeClipper::ID => Ok(Box::new(NativeClipper::new())),
            id if id == NativeExciter::ID => Ok(Box::new(NativeExciter::new())),
            id if id == NativeTape::ID => Ok(Box::new(NativeTape::new())),
            id if id == NativeSoundgoodizer::ID => Ok(Box::new(NativeSoundgoodizer::new())),
            id if id == NativeMonoFold::ID => Ok(Box::new(NativeMonoFold::new())),
            id if id == NativeRingMod::ID => Ok(Box::new(NativeRingMod::new())),
            id if id == NativeAutoFilter::ID => Ok(Box::new(NativeAutoFilter::new())),
            id if id == NativeStereoDouble::ID => Ok(Box::new(NativeStereoDouble::new())),
            id if id == NativeVocoder::ID => Ok(Box::new(NativeVocoder::new())),
            id if id == NativeStutter::ID => Ok(Box::new(NativeStutter::new())),
            id if id == NativeSampler::ID => Ok(Box::new(NativeSampler::new())),
            other => Err(format!("Unknown native plug-in id: {other}")),
        };
    }
    // Everything past here is someone else's C++ about to be loaded into this
    // process. A plug-in that crashes on load takes the DAW with it, usually
    // while a project is opening, which looks like the DAW losing the song.
    // Try it in a throwaway process first; a plug-in that kills that one is
    // refused by name instead of being loaded here.
    let verdict = crate::plugin_probe::verdict_for(&descriptor.path);
    if !verdict.is_safe_to_load() {
        let reason = verdict
            .message(&descriptor.name)
            .unwrap_or_else(|| format!("{} could not be loaded", descriptor.name));
        log::warn!("refusing to load {}: {reason}", descriptor.name);
        return Err(reason);
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

/// Off the main thread: a sync command runs on the window's own thread, so
/// a scan of a full plug-in folder froze the window until it ended.
#[tauri::command]
pub async fn scan_plugins(app: AppHandle) -> Result<Vec<PluginDescriptor>, String> {
    tauri::async_runtime::spawn_blocking(move || scan_plugins_blocking(app))
        .await
        .map_err(|e| format!("plug-in scan stopped: {e}"))
}

fn scan_plugins_blocking(app: AppHandle) -> Vec<PluginDescriptor> {
    // Hold the ENGINE lock only long enough to clone the scanner Arc, and
    // the scanner's own lock only at the start and end of the scan.
    let scanner_arc = {
        let state = app.state::<AppState>();
        let engine = state.engine.lock();
        std::sync::Arc::clone(&engine.plugin_scanner)
    };
    let emitter = app.clone();
    let progress: hardwave_plugin_host::scanner::ScanProgress = Box::new(move |count, label| {
        let _ = emitter.emit(
            "daw:pluginScanProgress",
            serde_json::json!({ "count": count, "current": label }),
        );
    });
    let result = hardwave_plugin_host::PluginScanner::scan_shared(&scanner_arc, Some(progress));
    let _ = app.emit(
        "daw:pluginScanComplete",
        serde_json::json!({ "count": result.len() }),
    );
    if let Some(path) = hardwave_plugin_host::PluginScanner::default_cache_path() {
        if let Err(e) = scanner_arc.lock().save_cache_to_disk(&path) {
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

/// Rescan plugin folders and bring back any project slots whose plugin
/// was missing but is now installed. Only PREVIOUSLY-missing slots are
/// (re)instantiated — `InsertCommand::Add` appends without dedup, so a
/// blanket re-hydrate would double every already-loaded plugin.
/// Returns the slots that are STILL missing after the rescan, so the
/// banner can update in place.
#[tauri::command]
pub fn rescan_and_restore_missing_plugins(
    app: AppHandle,
    state: State<AppState>,
) -> Result<Vec<MissingPluginInfo>, String> {
    // Snapshot which slots are missing BEFORE the rescan — these are
    // exactly the ones safe to Add if the scan finds their plugin.
    let missing_before = find_missing_plugins(state.clone());
    if missing_before.is_empty() {
        return Ok(Vec::new());
    }

    scan_plugins_blocking(app.clone());

    // Collect restore plan under the locks, instantiate outside them
    // (same discipline as hydrate_chains_from_project).
    // (track_id, slot_id, descriptor, enabled, wet, saved_state)
    type RestorePlanEntry = (String, String, PluginDescriptor, bool, f32, Option<Vec<u8>>);
    let plan: Vec<RestorePlanEntry> = {
        let engine = state.engine.lock();
        let scanner = engine.plugin_scanner.lock();
        let project = engine.project.lock();
        let mut acc = Vec::new();
        for m in &missing_before {
            let Some(descriptor) = scanner.find(&m.plugin_id) else {
                continue; // still missing
            };
            let Some(track) = project.tracks.iter().find(|t| t.id == m.track_id) else {
                continue;
            };
            let Some(slot) = track.inserts.iter().find(|s| s.id == m.slot_id) else {
                continue;
            };
            let saved_state = project.plugin_state(&slot.id).map(|e| e.chunk.clone());
            acc.push((
                m.track_id.clone(),
                slot.id.clone(),
                descriptor.clone(),
                slot.enabled,
                slot.wet,
                saved_state,
            ));
        }
        acc
    };

    for (track_id, slot_id, descriptor, enabled, wet, saved_state) in plan {
        match instantiate_for_slot(&state.sandboxed_plugins, &descriptor) {
            Ok(mut plugin) => {
                if let Some(bytes) = saved_state {
                    if let Err(e) = plugin.set_state(&bytes) {
                        log::warn!("rescan-restore: set_state failed for {slot_id}: {e}");
                    }
                }
                if let Some(queue) = plugin.pending_params() {
                    state
                        .slot_param_queues
                        .lock()
                        .insert((track_id.clone(), slot_id.clone()), queue);
                }
                if let Some(log) = plugin.gui_edit_log() {
                    state
                        .slot_gui_edit_logs
                        .lock()
                        .insert((track_id.clone(), slot_id.clone()), log);
                }
                let gain_reduction_db = LiveSlot::new_gain_reduction();
                state.slot_gain_reduction.lock().insert(
                    (track_id.clone(), slot_id.clone()),
                    gain_reduction_db.clone(),
                );
                let slot_levels =
                    std::sync::Arc::new(hardwave_engine::insert_chain::SlotLevels::with_params(
                        LiveSlot::ranges_of(plugin.as_ref())
                            .into_iter()
                            .map(|(id, _, _)| id),
                    ));
                state
                    .slot_levels
                    .lock()
                    .insert((track_id.clone(), slot_id.clone()), slot_levels.clone());
                let cmd = InsertCommand::Add {
                    track_id,
                    slot: LiveSlot {
                        param_ranges: LiveSlot::ranges_of(plugin.as_ref()),
                        levels: slot_levels.clone(),
                        slot_id,
                        plugin,
                        enabled,
                        wet,
                        sidechain_active: false,
                        gain_reduction_db,
                    },
                };
                if state.engine.lock().try_send_insert_command(cmd).is_err() {
                    return Err("insert command queue full — try again".into());
                }
            }
            Err(e) => log::warn!("rescan-restore: failed to load {}: {e}", descriptor.id),
        }
    }

    Ok(find_missing_plugins(state))
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
pub async fn open_plugin_editor(
    app: AppHandle,
    plugin_id: String,
    window_label: String,
    track_id: Option<String>,
    slot_id: Option<String>,
) -> Result<String, String> {
    // On Windows the window shows our bar above the plug-in, which is a page
    // of the app, so the label sits inside the panel-* permission. One
    // window per slot.
    #[cfg(windows)]
    let window_label = {
        let key: String = slot_id
            .as_deref()
            .unwrap_or(&window_label)
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(64)
            .collect();
        format!("panel-pluginEditor-{key}")
    };
    // A second click on Show GUI brings the open window forward. This was
    // checked after a whole new plug-in instance had been loaded, which
    // was then thrown away.
    if let Some(existing) = app.get_webview_window(&window_label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(window_label);
    }

    let (descriptor, shared_queue) = {
        let state = app.state::<AppState>();
        let descriptor = {
            let engine = state.engine.lock();
            let scanner = engine.plugin_scanner.lock();
            scanner
                .find(&plugin_id)
                .ok_or_else(|| format!("Plugin not found: {}", plugin_id))?
                .clone()
        };
        // With the slot's chain queue, knob moves in the window reach the
        // audio chain; without one the window is on its own.
        let shared_queue = match (track_id.as_deref(), slot_id.as_deref()) {
            (Some(t), Some(s)) => state
                .slot_param_queues
                .lock()
                .get(&(t.to_string(), s.to_string()))
                .cloned(),
            _ => None,
        };
        (descriptor, shared_queue)
    };

    // Built-in plug-ins have no window of their own: the mixer shows their
    // controls. Asked for one, this tried to load "<native>" as a CLAP file.
    if descriptor.path == std::path::Path::new("<native>") {
        return Err(format!(
            "{} is built in; its controls open in the mixer (Show controls)",
            descriptor.name
        ));
    }

    // The window needs the plug-in in this process; only one the probe
    // passed and the person has not sandboxed gets there.
    may_load_in_process(&descriptor)?;

    // The window's plug-in is a second instance beside the one that plays.
    // It opened at the plug-in's defaults, so the window showed settings
    // the song was not using. It starts from the playing slot's state.
    let slot_state = match (track_id.clone(), slot_id.clone()) {
        (Some(t), Some(s)) => {
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                app.state::<AppState>()
                    .engine
                    .lock()
                    .snapshot_plugin_states(std::time::Duration::from_millis(500))
                    .and_then(|mut states| states.remove(&(t, s)))
            })
            .await
            .ok()
            .flatten()
        }
        _ => None,
    };

    // Built here, from an async command, like the panel windows. As a sync
    // command this ran inside WebView2's own message callback: a window
    // made there never started its page, and a plug-in whose editor is a
    // WebView2 (every Hardwave plug-in) waited on it for ever. Show GUI did
    // nothing.
    let builder = {
        // Windows: the app's own page, which draws the bar with the
        // plug-in's name and presets; the plug-in goes in an area below it
        // (plugin_window_host). Elsewhere the plug-in fills the window.
        #[cfg(windows)]
        {
            let q = |v: &str| -> String {
                v.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
                    .collect::<String>()
                    .replace(' ', "%20")
            };
            let params = format!(
                "trackId={}&slotId={}&pluginId={}&name={}",
                q(track_id.as_deref().unwrap_or("")),
                q(slot_id.as_deref().unwrap_or("")),
                q(&plugin_id),
                q(&descriptor.name)
            );
            let init = format!(
                "(function(){{window.__HW_PANEL__={{panel:\"pluginHeader\",params:\"{params}\"}};}})();"
            );
            let url = app
                .get_webview_window("main")
                .and_then(|w| w.url().ok())
                .map(tauri::WebviewUrl::External)
                .unwrap_or_else(|| tauri::WebviewUrl::App("index.html".into()));
            tauri::WebviewWindowBuilder::new(&app, &window_label, url).initialization_script(&init)
        }
        #[cfg(not(windows))]
        {
            tauri::WebviewWindowBuilder::new(
                &app,
                &window_label,
                tauri::WebviewUrl::App("about:blank".into()),
            )
        }
    };
    let editor_window = builder
        .title(descriptor.name.clone())
        .inner_size(600.0, 400.0 + crate::plugin_window_host::HEADER_CSS_PX)
        .resizable(true)
        .always_on_top(true)
        .build()
        .map_err(|e| format!("Failed to open editor window: {e}"))?;

    // The plug-in and its GUI on the main thread, where a plug-in GUI has
    // to live, posted to it rather than run inside a callback.
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    {
        let app = app.clone();
        let window = editor_window.clone();
        let label = window_label.clone();
        let descriptor = descriptor.clone();
        app.clone()
            .run_on_main_thread(move || {
                let attach = || -> Result<(), String> {
                    let mut hosted: Box<dyn HostedPlugin> = match descriptor.format {
                        PluginFormat::Vst3 => Box::new(
                            match shared_queue {
                                Some(queue) => Vst3PluginInstance::load_with_shared_pending(
                                    descriptor.clone(),
                                    queue,
                                ),
                                None => Vst3PluginInstance::load(descriptor.clone()),
                            }
                            .map_err(|e| e.to_string())?,
                        ),
                        PluginFormat::Clap => Box::new(
                            match shared_queue {
                                Some(queue) => ClapPluginInstance::load_with_shared_pending(
                                    descriptor.clone(),
                                    queue,
                                ),
                                None => ClapPluginInstance::load(descriptor.clone()),
                            }
                            .map_err(|e| e.to_string())?,
                        ),
                    };
                    if let Some(bytes) = slot_state.as_deref() {
                        if let Err(e) = hosted.set_state(bytes) {
                            log::warn!(
                                "{}: window could not take the slot's state: {e}",
                                descriptor.name
                            );
                        }
                    }
                    #[cfg(windows)]
                    let (parent, area) = crate::plugin_window_host::make_area(&window)?;
                    #[cfg(not(windows))]
                    let parent = window
                        .window_handle()
                        .map_err(|e| format!("window handle unavailable: {e}"))?
                        .as_raw();
                    if !hosted.open_editor(parent) {
                        return Err(format!(
                            "{} has no window this host can show",
                            descriptor.name
                        ));
                    }
                    // The window takes the size the editor asks for, not
                    // a fixed 600 by 400 that cut most editors off.
                    let size = hosted.editor_size();
                    #[cfg(windows)]
                    {
                        let (w, h) = size.unwrap_or((600, 400));
                        area.fit(&window, w, h);
                    }
                    #[cfg(not(windows))]
                    if let Some((w, h)) = size {
                        let _ = window.set_size(tauri::PhysicalSize::new(w, h));
                    }
                    app.state::<AppState>()
                        .plugin_editors
                        .lock()
                        .insert(label.clone(), hosted);
                    Ok(())
                };
                let _ = tx.send(attach());
            })
            .map_err(|e| format!("could not reach the main thread: {e}"))?;
    }
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(std::time::Duration::from_secs(30))
            .unwrap_or_else(|_| Err("the plug-in took too long to open its window".into()))
    })
    .await
    .unwrap_or_else(|e| Err(format!("editor open stopped: {e}")));
    if let Err(e) = outcome {
        let _ = editor_window.close();
        return Err(e);
    }

    // Closing the window closes the plug-in's GUI with it. Nothing did
    // this before, so the app kept a GUI attached to a window that was gone.
    {
        let app = app.clone();
        let label = window_label.clone();
        editor_window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(mut hosted) =
                    app.state::<AppState>().plugin_editors.lock().remove(&label)
                {
                    hosted.close_editor();
                }
            }
        });
    }
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

/// The plug-ins the person chose to run sandboxed, shared with the
/// paths that load plug-ins without the app state at hand (export).
static SANDBOXED: std::sync::OnceLock<
    std::sync::Arc<parking_lot::Mutex<std::collections::HashSet<String>>>,
> = std::sync::OnceLock::new();

/// Make the sandboxed set, once, and keep it where every loader finds it.
pub(crate) fn sandboxed_set(
    initial: std::collections::HashSet<String>,
) -> std::sync::Arc<parking_lot::Mutex<std::collections::HashSet<String>>> {
    std::sync::Arc::clone(
        SANDBOXED.get_or_init(|| std::sync::Arc::new(parking_lot::Mutex::new(initial))),
    )
}

/// Load a plug-in for any purpose (a slot, the parameter sheet, presets,
/// an export), honouring the crash probe and the sandbox choice. The one
/// way in: nothing loads a third-party plug-in around it.
pub(crate) fn load_hosted(descriptor: &PluginDescriptor) -> Result<Box<dyn HostedPlugin>, String> {
    let sandboxed = sandboxed_set(Default::default());
    instantiate_for_slot(&sandboxed, descriptor)
}

/// Whether this plug-in may be loaded into the DAW's own process: not
/// refused by the probe, not sandboxed by choice, built for this
/// architecture. Its own window needs that, because a window cannot be
/// handed between processes here.
pub(crate) fn may_load_in_process(descriptor: &PluginDescriptor) -> Result<(), String> {
    if descriptor.path == std::path::Path::new("<native>") {
        return Ok(());
    }
    if sandboxed_set(Default::default())
        .lock()
        .contains(&descriptor.id)
    {
        return Err(format!(
            "{} runs in its own process, so its own window is not available; its parameters are",
            descriptor.name
        ));
    }
    if !hardwave_plugin_host::binary_arch::plugin_arch(&descriptor.path).matches_host() {
        return Err(format!(
            "{} is built for another kind of computer and runs in a helper, so its own window is not available",
            descriptor.name
        ));
    }
    let verdict = crate::plugin_probe::verdict_for(&descriptor.path);
    if !verdict.is_safe_to_load() {
        return Err(verdict
            .message(&descriptor.name)
            .unwrap_or_else(|| format!("{} could not be loaded", descriptor.name)));
    }
    Ok(())
}

/// Build the plug-in for a slot, in a process of its own when the user
/// has asked for that one to be sandboxed.
///
/// A sandboxed plug-in costs a block of latency, which is why it is a
/// choice rather than the default: what it buys is that its crash is
/// its own process's problem and the song keeps playing.
pub(crate) fn instantiate_for_slot(
    sandboxed_plugins: &std::sync::Arc<parking_lot::Mutex<std::collections::HashSet<String>>>,
    descriptor: &PluginDescriptor,
) -> Result<Box<dyn HostedPlugin>, String> {
    let sandboxed = sandboxed_plugins.lock().contains(&descriptor.id);
    // A plug-in built for another architecture has no choice: this
    // process cannot load it, so it goes out to a helper whether or
    // not the user asked for a sandbox.
    let foreign = !hardwave_plugin_host::binary_arch::plugin_arch(&descriptor.path).matches_host();
    if !sandboxed && !foreign {
        return instantiate_plugin(descriptor);
    }
    let exe =
        std::env::current_exe().map_err(|e| format!("cannot find our own executable: {e}"))?;
    // No falling back to this process: the person sandboxed this plug-in
    // because it crashes, and a sandbox that will not start does not
    // change that. The slot says why it is empty instead.
    crate::plugin_sandbox::SandboxedPlugin::start(descriptor.clone(), &exe)
        .map(|plugin| Box::new(plugin) as Box<dyn HostedPlugin>)
        .map_err(|e| {
            format!(
                "{} could not start in its own process: {e}",
                descriptor.name
            )
        })
}

pub(crate) fn add_plugin_to_track_quietly(
    state: State<AppState>,
    track_id: String,
    plugin_id: String,
) -> Result<String, String> {
    state.engine.lock().snapshot_before_mutation();
    add_plugin_without_undo_step(state, track_id, plugin_id)
}

/// Add a plug-in without an undo step of its own, for callers that add
/// many as one change and took the snapshot themselves.
pub(crate) fn add_plugin_without_undo_step(
    state: State<AppState>,
    track_id: String,
    plugin_id: String,
) -> Result<String, String> {
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
    // A plug-in that fails to load takes its slot back out: the slot was
    // left in the project with nothing behind it, an empty insert that
    // came back with every save.
    let plugin = match instantiate_for_slot(&state.sandboxed_plugins, &descriptor) {
        Ok(plugin) => plugin,
        Err(e) => {
            let engine = state.engine.lock();
            if let Some(track) = engine.project.lock().track_mut(&track_id) {
                track.inserts.retain(|slot| slot.id != slot_id);
            }
            return Err(e);
        }
    };

    // Phase 2b: capture the slot's parameter queue (VST3 + CLAP) BEFORE
    // shipping the plug-in to the audio thread. This lets the editor
    // path wire a fresh editor instance to the same queue so GUI knob
    // movements reach the audio chain.
    if let Some(queue) = plugin.pending_params() {
        state
            .slot_param_queues
            .lock()
            .insert((track_id.clone(), slot_id.clone()), queue);
    }
    // The second copy of those edits, the one the app drains, is what
    // lets automation record a knob inside the plug-in's own window.
    if let Some(log) = plugin.gui_edit_log() {
        state
            .slot_gui_edit_logs
            .lock()
            .insert((track_id.clone(), slot_id.clone()), log);
    }

    // Phase 3: ship the freshly-built LiveSlot to the audio thread via
    // the lock-free InsertCommand queue. The chain takes ownership and
    // calls activate() before processing the next block.
    let gain_reduction_db = LiveSlot::new_gain_reduction();
    state.slot_gain_reduction.lock().insert(
        (track_id.clone(), slot_id.clone()),
        gain_reduction_db.clone(),
    );
    let slot_levels = std::sync::Arc::new(hardwave_engine::insert_chain::SlotLevels::with_params(
        LiveSlot::ranges_of(plugin.as_ref())
            .into_iter()
            .map(|(id, _, _)| id),
    ));
    state
        .slot_levels
        .lock()
        .insert((track_id.clone(), slot_id.clone()), slot_levels.clone());
    let cmd = InsertCommand::Add {
        track_id: track_id.clone(),
        slot: LiveSlot {
            param_ranges: LiveSlot::ranges_of(plugin.as_ref()),
            levels: slot_levels.clone(),
            slot_id: slot_id.clone(),
            plugin,
            enabled: true,
            wet: 1.0,
            gain_reduction_db,
            // Synced to the project's sidechain_source on the next
            // graph rebuild; new inserts start with none.
            sidechain_active: false,
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

fn remove_plugin_from_track_body(
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
    state.slot_param_queues.lock().remove(&(track_id, slot_id));
    Ok(())
}

fn set_insert_enabled_body(
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

fn reorder_insert_body(
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

fn set_insert_wet_body(
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
) -> Result<Option<String>, String> {
    // The value as text, for the control that moved it.
    let text = parameter_text_for_slot(&state, &track_id, &slot_id, param_id, value);
    // The other side of a room hears the knob too (live state is not in
    // the track's saved state, which is what travels otherwise).
    state
        .collab
        .send(hardwave_project::multiplayer::SyncKind::PluginParam {
            track_id: track_id.clone(),
            slot_id: slot_id.clone(),
            param_id,
            value,
        });
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
    Ok(text)
}

/// Bring a track's live plug-ins in line with its slots after the track
/// was replaced (a room sent it): drop the ones it no longer has, load the
/// new ones in their places, and give changed ones their new state. Runs
/// where plug-ins are made (the main thread).
pub(crate) fn sync_track_chain(
    state: &AppState,
    track_id: &str,
    old: &[hardwave_project::track::PluginSlot],
    new: &[hardwave_project::track::PluginSlot],
) {
    let engine = state.engine.lock();
    let send = |cmd: InsertCommand| {
        if engine.try_send_insert_command(cmd).is_err() {
            log::warn!("room: insert queue full while syncing {track_id}");
        }
    };
    let same = |a: &hardwave_project::track::PluginSlot,
                b: &hardwave_project::track::PluginSlot| {
        a.id == b.id && a.plugin_id == b.plugin_id
    };
    // The live chain's order, as it will be after each step.
    let mut order: Vec<String> = Vec::new();
    for slot in old {
        if new.iter().any(|n| same(n, slot)) {
            order.push(slot.id.clone());
        } else {
            send(InsertCommand::Remove {
                track_id: track_id.to_string(),
                slot_id: slot.id.clone(),
            });
        }
    }
    for (index, slot) in new.iter().enumerate() {
        match old.iter().find(|o| same(o, slot)) {
            Some(prev) => {
                if slot.state.is_some() && slot.state != prev.state {
                    if let Some(bytes) = &slot.state {
                        send(InsertCommand::SetState {
                            track_id: track_id.to_string(),
                            slot_id: slot.id.clone(),
                            bytes: bytes.clone(),
                        });
                    }
                }
                if slot.enabled != prev.enabled {
                    send(InsertCommand::SetEnabled {
                        track_id: track_id.to_string(),
                        slot_id: slot.id.clone(),
                        enabled: slot.enabled,
                    });
                }
                if slot.wet != prev.wet {
                    send(InsertCommand::SetWet {
                        track_id: track_id.to_string(),
                        slot_id: slot.id.clone(),
                        wet: slot.wet,
                    });
                }
            }
            None => {
                let Some(descriptor) = engine.plugin_scanner.lock().find(&slot.plugin_id).cloned()
                else {
                    log::warn!("room: plug-in {} is not on this machine", slot.plugin_id);
                    continue;
                };
                let mut plugin = match instantiate_for_slot(&state.sandboxed_plugins, &descriptor) {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("room: could not load {}: {e}", slot.plugin_id);
                        continue;
                    }
                };
                if let Some(bytes) = &slot.state {
                    let _ = plugin.set_state(bytes);
                }
                let live = live_slot(
                    state,
                    track_id,
                    slot.id.clone(),
                    plugin,
                    slot.enabled,
                    slot.wet,
                    slot.sidechain_source.is_some(),
                );
                send(InsertCommand::Add {
                    track_id: track_id.to_string(),
                    slot: live,
                });
                order.push(slot.id.clone());
                let from = order.len() - 1;
                let to = index.min(from);
                if from != to {
                    let id = order.remove(from);
                    order.insert(to, id);
                    send(InsertCommand::Reorder {
                        track_id: track_id.to_string(),
                        from,
                        to,
                    });
                }
            }
        }
    }
}

/// One instance per plug-in kind, kept to turn values into text: a
/// parameter's text depends on the kind of plug-in, not on its settings.
fn text_probes(
) -> &'static parking_lot::Mutex<std::collections::HashMap<String, Box<dyn HostedPlugin>>> {
    static PROBES: std::sync::OnceLock<
        parking_lot::Mutex<std::collections::HashMap<String, Box<dyn HostedPlugin>>>,
    > = std::sync::OnceLock::new();
    PROBES.get_or_init(Default::default)
}

/// The plug-in id in a slot.
fn slot_plugin_id(state: &AppState, track_id: &str, slot_id: &str) -> Option<String> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project
        .track(track_id)?
        .inserts
        .iter()
        .find(|s| s.id == slot_id)
        .map(|s| s.plugin_id.clone())
}

fn parameter_text_for_slot(
    state: &AppState,
    track_id: &str,
    slot_id: &str,
    param_id: u32,
    value: f64,
) -> Option<String> {
    let plugin_id = slot_plugin_id(state, track_id, slot_id)?;
    // Only built-in plug-ins: loading a third-party one just to format a
    // number would be slow, and it is loaded in this process.
    if !plugin_id.starts_with("hardwave.native.") {
        return None;
    }
    let mut probes = text_probes().lock();
    if !probes.contains_key(&plugin_id) {
        let descriptor = state
            .engine
            .lock()
            .plugin_scanner
            .lock()
            .find(&plugin_id)
            .cloned()?;
        probes.insert(plugin_id.clone(), instantiate_plugin(&descriptor).ok()?);
    }
    probes.get(&plugin_id)?.parameter_text(param_id, value)
}

#[derive(Serialize)]
pub struct PluginParamInfo {
    pub id: u32,
    pub name: String,
    #[serde(rename = "defaultValue")]
    pub default_value: f64,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub unit: String,
    pub automatable: bool,
    /// The value as people read it, when the plug-in can say.
    pub text: Option<String>,
    /// The choices of a parameter that picks one of a few.
    pub options: Option<Vec<String>>,
    /// Built-ins only: the text at 101 even steps from min to max, so a
    /// window can name any position while it draws or drags without asking
    /// the app each time.
    pub texts: Option<Vec<String>>,
}

/// Enumerate a plug-in slot's parameters for the generic parameter sheet
/// (the fallback UI for plug-ins without their own editor). Resolves the
/// slot's descriptor and instantiates a throwaway instance to read the
/// static parameter metadata — names, ranges, defaults — without touching
/// the audio thread. Returned `value` is the parameter default; the UI
/// tracks live edits it pushes via `set_plugin_parameter`.
#[tauri::command]
pub fn get_plugin_parameters(
    state: State<AppState>,
    track_id: String,
    slot_id: String,
) -> Result<Vec<PluginParamInfo>, String> {
    let engine = state.engine.lock();
    let plugin_id = {
        let project = engine.project.lock();
        let track = project
            .track(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        track
            .inserts
            .iter()
            .find(|s| s.id == slot_id)
            .map(|s| s.plugin_id.clone())
            .ok_or_else(|| format!("Insert not found: {slot_id}"))?
    };
    let descriptor = {
        let scanner = engine.plugin_scanner.lock();
        scanner
            .find(&plugin_id)
            .cloned()
            .ok_or_else(|| format!("Plugin descriptor not found: {plugin_id}"))?
    };
    drop(engine);

    let mut plugin = load_hosted(&descriptor)?;
    // The values the slot is playing, not this instance's defaults: the
    // sheet opened at defaults and showed settings the song was not using.
    let live = state
        .engine
        .lock()
        .snapshot_plugin_states(std::time::Duration::from_millis(300))
        .and_then(|mut states| states.remove(&(track_id.clone(), slot_id.clone())));
    let saved = || {
        let engine = state.engine.lock();
        let project = engine.project.lock();
        project
            .track(&track_id)
            .and_then(|t| t.inserts.iter().find(|s| s.id == slot_id))
            .and_then(|s| s.state.clone())
    };
    if let Some(bytes) = live.or_else(saved) {
        let _ = plugin.set_state(&bytes);
    }
    let count = plugin.get_parameter_count();
    let built_in = plugin_id.starts_with("hardwave.native.");
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        if let Some(info) = plugin.get_parameter_info(i) {
            let value = plugin.get_parameter_value(info.id);
            let texts = built_in.then(|| {
                (0..=100)
                    .map(|k| {
                        let v = info.min + (info.max - info.min) * f64::from(k) / 100.0;
                        plugin.parameter_text(info.id, v).unwrap_or_default()
                    })
                    .collect()
            });
            out.push(PluginParamInfo {
                id: info.id,
                text: plugin.parameter_text(info.id, value),
                options: plugin.parameter_options(info.id),
                texts,
                name: info.name,
                default_value: info.default_value,
                value,
                min: info.min,
                max: info.max,
                unit: info.unit,
                automatable: info.automatable,
            });
        }
    }
    Ok(out)
}

/// How much each insert is pulling the level down right now, in dB.
///
/// The compressors and limiters have always worked this out and thrown it
/// away, so a mixer could not show what a compressor was doing: the only
/// way to tell was by ear against the meter.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotGainReduction {
    pub track_id: String,
    pub slot_id: String,
    /// Non-positive dB. Zero means the plug-in is not reducing anything.
    pub reduction_db: f32,
}

#[tauri::command]
pub fn get_gain_reduction(state: State<AppState>) -> Vec<SlotGainReduction> {
    use std::sync::atomic::Ordering;
    state
        .slot_gain_reduction
        .lock()
        .iter()
        .map(|((track_id, slot_id), value)| SlotGainReduction {
            track_id: track_id.clone(),
            slot_id: slot_id.clone(),
            reduction_db: value.load(Ordering::Relaxed),
        })
        .collect()
}

/// Hydrate every persisted PluginSlot in the current project/// Hydrate every persisted PluginSlot in the current project into the
/// audio thread's chains. Called from `load_project` after the project
/// state has been replaced. For each insert: instantiate the plug-in,
/// ship an Add command. Plug-ins missing from the scanner cache are
/// skipped with a warning so the project still opens — the user gets a
/// "missing plug-ins" notice via `find_missing_plugins`.
/// Load an audio file into a NativeSampler slot on a track. Decodes the
/// file here (off the audio thread), then ships the decoded PCM to the
/// plug-in as its state via the existing SetState path — and stores the
/// same bytes in the project so the sample persists across save/reload
/// and is applied on export (offline hydrate). The track must already
/// hold the sampler insert (added via `add_plugin_to_track`).
#[tauri::command]
pub fn load_sampler(
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    path: String,
    base_note: Option<u8>,
) -> Result<(), String> {
    // Loading a sample into a sampler is an edit: undo has to bring back
    // whatever the slot held before.
    state.engine.lock().snapshot_before_mutation();
    let (info, channels) =
        hardwave_dsp::audio_file::AudioFileReader::read(std::path::Path::new(&path))
            .map_err(|e| format!("decode {path}: {e:?}"))?;
    let base = base_note.unwrap_or(60);
    let bytes =
        hardwave_native_plugins::sampler::encode_sample_state(info.sample_rate, base, channels);
    // Persist in the project (survives rebuild/reload + offline export).
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        project.set_plugin_state(slot_id.clone(), "native-sampler", bytes.clone());
    }
    // Ship to the live chain instance.
    let cmd = InsertCommand::SetState {
        track_id,
        slot_id,
        bytes,
    };
    state
        .engine
        .lock()
        .try_send_insert_command(cmd)
        .map_err(|_| "insert command queue full or engine not started".to_string())?;
    Ok(())
}

pub fn hydrate_chains_from_project(state: &AppState) -> Result<(), String> {
    hydrate_chains(state, None)
}

/// Load the plug-ins of one track into the engine (a cloned channel).
pub fn hydrate_track_chain(state: &AppState, track_id: &str) -> Result<(), String> {
    hydrate_chains(state, Some(track_id))
}

#[allow(clippy::type_complexity)]
fn hydrate_chains(state: &AppState, only_track: Option<&str>) -> Result<(), String> {
    // Snapshot what we need under the locks, then drop them before we
    // start instantiating plug-ins (slow VST3 / CLAP loads). We also
    // collect any saved plug-in state chunks here so we can restore
    // them onto the fresh instance below — without this step every
    // load() round-trip silently reset plug-in knobs to defaults
    // (`PluginSlot.state` was being serialized but never replayed).
    let plan: Vec<(
        String,
        String,
        PluginDescriptor,
        bool,
        f32,
        Option<Vec<u8>>,
        bool,
    )> = {
        let engine = state.engine.lock();
        let project = engine.project.lock();
        let scanner = engine.plugin_scanner.lock();
        let mut acc = Vec::new();
        for track in project
            .tracks
            .iter()
            .filter(|t| only_track.is_none_or(|id| t.id == id))
        {
            for slot in &track.inserts {
                if let Some(descriptor) = scanner.find(&slot.plugin_id) {
                    let saved_state = project
                        .plugin_state(&slot.id)
                        .map(|entry| entry.chunk.clone());
                    acc.push((
                        track.id.clone(),
                        slot.id.clone(),
                        descriptor.clone(),
                        slot.enabled,
                        slot.wet,
                        saved_state,
                        slot.sidechain_source.is_some(),
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

    for (track_id, slot_id, descriptor, enabled, wet, saved_state, sidechain_active) in plan {
        match instantiate_for_slot(&state.sandboxed_plugins, &descriptor) {
            Ok(mut plugin) => {
                // Restore the persisted state BEFORE the plug-in joins
                // the audio chain — once it's on the audio thread we
                // have no synchronous way to push state into it.
                if let Some(bytes) = saved_state {
                    if let Err(e) = plugin.set_state(&bytes) {
                        log::warn!(
                            "hydrate: set_state failed for {} on track {}: {e}",
                            slot_id,
                            track_id
                        );
                    }
                }
                let slot = live_slot(
                    state,
                    &track_id,
                    slot_id,
                    plugin,
                    enabled,
                    wet,
                    sidechain_active,
                );
                let cmd = InsertCommand::Add { track_id, slot };
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

fn set_plugin_sidechain_source_body(
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

fn set_fx_chain_bypassed_body(
    state: State<AppState>,
    track_id: String,
    bypassed: bool,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let slot_ids: Vec<String> = {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {}", track_id))?;
        for slot in &mut track.inserts {
            slot.enabled = !bypassed;
        }
        track.inserts.iter().map(|s| s.id.clone()).collect()
    };

    // Push the change to the audio thread explicitly, the way
    // `set_insert_enabled` does. Mutating the project and rebuilding is not
    // enough: `rebuild_graph` stashes and restores live insert chains verbatim
    // and only re-syncs the sidechain flag, so `enabled` never reached the
    // chain. The result was a bypass that did nothing during playback while
    // the export — which reads `enabled` straight off the project — dropped
    // all of that track's FX.
    {
        let engine = state.engine.lock();
        for slot_id in slot_ids {
            let _ = engine.try_send_insert_command(InsertCommand::SetEnabled {
                track_id: track_id.clone(),
                slot_id,
                enabled: !bypassed,
            });
        }
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Try a plug-in again after it was refused for crashing on load.
///
/// The verdict is remembered per plug-in version, so a reinstall or update is
/// picked up on its own. This is for the case where someone fixed the
/// installation without the file changing, and for a plug-in manager button
/// that says so out loud rather than making the user restart the DAW.
#[tauri::command]
pub fn retry_blocked_plugin(path: String) {
    crate::plugin_probe::retry(std::path::Path::new(&path));
}

/// One knob move made inside a plug-in's own window.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginKnobMove {
    pub track_id: String,
    pub slot_id: String,
    pub param_id: u32,
    /// 0 to 1, the same shape an automation lane stores.
    pub value: f64,
}

/// Take the knob moves made in plug-in windows since the last call.
///
/// Faders and the generic parameter sheet have recorded automation
/// since v0.226 because the app is the one moving them. A knob inside a
/// plug-in's own window is moved by the plug-in, and the queue that
/// carried those moves was drained by the audio thread before the app
/// could see it. The host keeps a second copy now, and this hands it
/// over so automation write can follow it like any other control.
#[tauri::command]
pub fn drain_plugin_knob_moves(state: State<AppState>) -> Vec<PluginKnobMove> {
    let logs = state.slot_gui_edit_logs.lock();
    let mut moves = Vec::new();
    for ((track_id, slot_id), log) in logs.iter() {
        let taken: Vec<(u32, f64)> = {
            let mut q = log.lock();
            std::mem::take(&mut *q)
        };
        for (param_id, value) in taken {
            moves.push(PluginKnobMove {
                track_id: track_id.clone(),
                slot_id: slot_id.clone(),
                param_id,
                value: value.clamp(0.0, 1.0),
            });
        }
    }
    moves
}

/// Which plug-ins run in a process of their own.
#[tauri::command]
pub fn get_sandboxed_plugins(state: State<AppState>) -> Vec<String> {
    let mut list: Vec<String> = state.sandboxed_plugins.lock().iter().cloned().collect();
    list.sort();
    list
}

/// Run this plug-in in a process of its own, or stop doing that.
///
/// It takes effect the next time the plug-in is loaded: swapping a
/// live one for a sandboxed copy mid-playback would drop whatever it
/// is holding, which is worse than waiting for the next load.
#[tauri::command]
pub fn set_plugin_sandboxed(
    state: State<AppState>,
    plugin_id: String,
    sandboxed: bool,
) -> Result<bool, String> {
    {
        let mut list = state.sandboxed_plugins.lock();
        if sandboxed {
            list.insert(plugin_id);
        } else {
            list.remove(&plugin_id);
        }
    }
    crate::commands::engine::persist_audio_prefs_public(&state);
    Ok(sandboxed)
}

/// A plug-in whose own process has gone.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxCrash {
    pub plugin_id: String,
    pub message: String,
}

/// Which sandboxed plug-ins have crashed since the app last asked.
///
/// A crash used to close the DAW. Now the slot goes quiet and the song
/// keeps playing, which is only useful if the app says which plug-in
/// it was, so this is polled and reported once.
#[tauri::command]
pub fn take_sandbox_crashes(_state: State<AppState>) -> Vec<SandboxCrash> {
    let crashes: Vec<SandboxCrash> = crate::plugin_sandbox::crashed_sandboxes()
        .into_iter()
        .map(|(plugin_id, message)| SandboxCrash { plugin_id, message })
        .collect();
    if !crashes.is_empty() {
        crate::plugin_sandbox::clear_crashed_sandboxes();
    }
    crashes
}

/// Every window holds its own copy of the track list. A plug-in added in
/// the detached mixer did not show in the main window (or the other way
/// round) until that window happened to reload, which looked like the
/// plug-in moving between channels. Each change to a channel's inserts now
/// says which channel, to every window, and each reloads that channel.
pub(crate) fn tell_windows_track_changed(app: &AppHandle, track_id: &str) {
    let _ = app.emit(
        "daw:trackChanged",
        serde_json::json!({ "trackId": track_id }),
    );
}

#[tauri::command]
pub fn add_plugin_to_track(
    app: AppHandle,
    state: State<AppState>,
    track_id: String,
    plugin_id: String,
) -> Result<String, String> {
    let result = add_plugin_to_track_quietly(state, track_id.clone(), plugin_id);
    if result.is_ok() {
        tell_windows_track_changed(&app, &track_id);
    }
    result
}

#[tauri::command]
pub fn remove_plugin_from_track(
    app: AppHandle,
    state: State<AppState>,
    track_id: String,
    slot_id: String,
) -> Result<(), String> {
    let result = remove_plugin_from_track_body(state, track_id.clone(), slot_id);
    if result.is_ok() {
        tell_windows_track_changed(&app, &track_id);
    }
    result
}

#[tauri::command]
pub fn set_insert_enabled(
    app: AppHandle,
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    enabled: bool,
) -> Result<(), String> {
    let result = set_insert_enabled_body(state, track_id.clone(), slot_id, enabled);
    if result.is_ok() {
        tell_windows_track_changed(&app, &track_id);
    }
    result
}

#[tauri::command]
pub fn reorder_insert(
    app: AppHandle,
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    new_index: usize,
) -> Result<(), String> {
    let result = reorder_insert_body(state, track_id.clone(), slot_id, new_index);
    if result.is_ok() {
        tell_windows_track_changed(&app, &track_id);
    }
    result
}

#[tauri::command]
pub fn set_insert_wet(
    app: AppHandle,
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    wet: f32,
) -> Result<(), String> {
    let result = set_insert_wet_body(state, track_id.clone(), slot_id, wet);
    if result.is_ok() {
        tell_windows_track_changed(&app, &track_id);
    }
    result
}

#[tauri::command]
pub fn set_plugin_sidechain_source(
    app: AppHandle,
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    source_track_id: Option<String>,
) -> Result<(), String> {
    let result =
        set_plugin_sidechain_source_body(state, track_id.clone(), slot_id, source_track_id);
    if result.is_ok() {
        tell_windows_track_changed(&app, &track_id);
    }
    result
}

#[tauri::command]
pub fn set_fx_chain_bypassed(
    app: AppHandle,
    state: State<AppState>,
    track_id: String,
    bypassed: bool,
) -> Result<(), String> {
    let result = set_fx_chain_bypassed_body(state, track_id.clone(), bypassed);
    if result.is_ok() {
        tell_windows_track_changed(&app, &track_id);
    }
    result
}

/// A wavetable bank as the built-in wavetable synth plays it: `positions`
/// frames across the table, `points` samples each, for its window to draw.
#[tauri::command]
pub fn wavetable_frames(bank: String, positions: usize, points: usize) -> Vec<Vec<f32>> {
    use hardwave_dsp::wavetable::Wavetable;
    let table = match bank.as_str() {
        "Basic" => Wavetable::basic(),
        "Analog" => Wavetable::analog(),
        "Digital" => Wavetable::digital(),
        "Vocal" => Wavetable::vocal(),
        _ => Wavetable::noise(),
    };
    let positions = positions.clamp(1, 64);
    let points = points.clamp(8, 512);
    (0..positions)
        .map(|k| {
            let position = if positions == 1 {
                0.0
            } else {
                k as f32 / (positions - 1) as f32
            };
            (0..points)
                .map(|i| table.sample(i as f32 / points as f32, position))
                .collect()
        })
        .collect()
}

/// What a plug-in's window shows live, in one call: the peak levels in and
/// out since the last call ([in L, in R, out L, out R], linear), the gain
/// reduction, the last output frames when `scope` is asked for, and the
/// parameters the song moved (automation, a controller, modulation) so the
/// knobs follow.
///
/// It was four calls every 33 ms per open window, each one a round trip
/// through the main thread, which is shared with everything else the
/// interface asks for: the window's meters and knobs fell behind the
/// sound. One call, run off the main thread, since it only reads.
#[derive(serde::Serialize, Default)]
pub struct SlotLive {
    pub levels: [f32; 4],
    pub gr: Option<f32>,
    pub scope: Option<Vec<f32>>,
    pub changes: Vec<(u32, f64)>,
    /// The sound going into and coming out of the slot, as SPECTRUM_BANDS
    /// log-spaced bands from 20 Hz to 20 kHz in dBFS, when asked for: an
    /// EQ's analyzer shows both.
    pub spectrum: Option<(Vec<f32>, Vec<f32>)>,
}

/// How many bands the analyzer gets: enough for a smooth curve across a
/// window a thousand pixels wide, few enough to send 30 times a second.
const SPECTRUM_BANDS: usize = 192;

#[tauri::command(async)]
pub fn get_slot_live(
    state: State<'_, AppState>,
    track_id: String,
    slot_id: String,
    scope: bool,
    spectrum: Option<bool>,
) -> SlotLive {
    use std::sync::atomic::Ordering;
    let key = (track_id, slot_id);
    let gr = state
        .slot_gain_reduction
        .lock()
        .get(&key)
        .map(|v| v.load(Ordering::Relaxed));
    let levels = state.slot_levels.lock().get(&key).cloned();
    let Some(levels) = levels else {
        return SlotLive {
            gr,
            ..SlotLive::default()
        };
    };
    let spectrum = spectrum.unwrap_or(false).then(|| {
        let (pre, post, rate) = levels.spectrum_samples();
        let bands = |s: &[f32]| {
            hardwave_dsp::spectrum::log_band_spectrum_db(
                s,
                rate as f32,
                SPECTRUM_BANDS,
                20.0,
                20_000.0,
            )
        };
        (bands(&pre), bands(&post))
    });
    SlotLive {
        levels: levels.take(),
        gr,
        scope: scope.then(|| levels.scope()),
        changes: levels.take_param_changes(),
        spectrum,
    }
}

/// A plug-in instance ready to join a chain, with its gain reduction and
/// levels registered for the windows that read them.
fn live_slot(
    state: &AppState,
    track_id: &str,
    slot_id: String,
    plugin: Box<dyn HostedPlugin>,
    enabled: bool,
    wet: f32,
    sidechain_active: bool,
) -> LiveSlot {
    let key = (track_id.to_string(), slot_id.clone());
    let gain_reduction_db = LiveSlot::new_gain_reduction();
    state
        .slot_gain_reduction
        .lock()
        .insert(key.clone(), gain_reduction_db.clone());
    let ranges = LiveSlot::ranges_of(plugin.as_ref());
    let levels = std::sync::Arc::new(hardwave_engine::insert_chain::SlotLevels::with_params(
        ranges.iter().map(|(id, _, _)| *id),
    ));
    state.slot_levels.lock().insert(key, levels.clone());
    LiveSlot {
        param_ranges: ranges,
        levels,
        slot_id,
        plugin,
        enabled,
        wet,
        sidechain_active,
        gain_reduction_db,
    }
}

/// Start a crashed plug-in again: every slot holding it gets a fresh
/// instance in its own process, with the settings saved for that slot, in
/// the same place in its chain. A crash used to leave the slot silent until
/// the plug-in was removed and added by hand. Returns how many restarted.
#[tauri::command]
pub fn restart_sandboxed_plugin(
    app: AppHandle,
    state: State<AppState>,
    plugin_id: String,
) -> Result<usize, String> {
    #[allow(clippy::type_complexity)]
    let (descriptor, plan): (
        PluginDescriptor,
        Vec<(
            String,
            String,
            usize,
            usize,
            bool,
            f32,
            bool,
            Option<Vec<u8>>,
        )>,
    ) = {
        let engine = state.engine.lock();
        let descriptor = engine
            .plugin_scanner
            .lock()
            .find(&plugin_id)
            .cloned()
            .ok_or_else(|| format!("Plug-in not found: {plugin_id}"))?;
        let project = engine.project.lock();
        let mut plan = Vec::new();
        for track in &project.tracks {
            for (index, slot) in track.inserts.iter().enumerate() {
                if slot.plugin_id == plugin_id {
                    plan.push((
                        track.id.clone(),
                        slot.id.clone(),
                        index,
                        track.inserts.len(),
                        slot.enabled,
                        slot.wet,
                        slot.sidechain_source.is_some(),
                        project
                            .plugin_state(&slot.id)
                            .map(|e| e.chunk.clone())
                            .or_else(|| slot.state.clone()),
                    ));
                }
            }
        }
        (descriptor, plan)
    };
    let mut restarted = 0;
    for (track_id, slot_id, index, count, enabled, wet, sidechain_active, saved) in plan {
        let mut plugin = instantiate_for_slot(&state.sandboxed_plugins, &descriptor)?;
        if let Some(bytes) = saved {
            let _ = plugin.set_state(&bytes);
        }
        let slot = live_slot(
            &state,
            &track_id,
            slot_id.clone(),
            plugin,
            enabled,
            wet,
            sidechain_active,
        );
        let engine = state.engine.lock();
        // Out with the dead one, in with the new one at the end of the
        // chain, then back to where the old one sat.
        for cmd in [
            InsertCommand::Remove {
                track_id: track_id.clone(),
                slot_id,
            },
            InsertCommand::Add {
                track_id: track_id.clone(),
                slot,
            },
            InsertCommand::Reorder {
                track_id: track_id.clone(),
                from: count.saturating_sub(1),
                to: index,
            },
        ] {
            engine
                .try_send_insert_command(cmd)
                .map_err(|_| "insert command queue full; try again".to_string())?;
        }
        drop(engine);
        tell_windows_track_changed(&app, &track_id);
        restarted += 1;
    }
    Ok(restarted)
}
