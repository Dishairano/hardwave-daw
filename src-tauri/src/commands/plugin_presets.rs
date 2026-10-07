//! Plug-in user-preset management.
//!
//! Lets the user save the current state of a hosted plug-in to disk
//! under a friendly name, browse the saved list, and load any preset
//! back into the live slot. Implements the "Presets" affordance the
//! FL Studio manual describes as right-click double arrows in the
//! plug-in wrapper (manual page "The User Interface", Presets section).
//!
//! Disk layout under `<appdata>/hardwave/presets/`:
//!
//!   <plugin-id-safe>/
//!     index.json      — Vec<PresetInfo> ordered most-recently-created first
//!     <preset-id>.bin — raw state bytes (the same blob `get_state()` returns)
//!
//! Factory-preset enumeration (VST3 IProgramListData / IUnitInfo) isn't
//! wired yet — that lands when the plug-in host crate exposes program
//! info. For now we only deal with user-saved presets.

use crate::AppState;
use hardwave_engine::insert_chain::InsertCommand;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetInfo {
    pub id: String,
    pub name: String,
    pub created_at: u64,
}

/// Convert a plug-in id (which may contain `/`, `:`, etc.) into a
/// filesystem-safe directory name. Lowercases + replaces anything that
/// isn't `[a-z0-9._-]` with `_`.
fn sanitize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '.' | '-' | '_' => c,
            _ => '_',
        })
        .collect()
}

fn preset_dir(app: &AppHandle, plugin_id: &str) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?;
    let dir = base
        .join("hardwave")
        .join("presets")
        .join(sanitize(plugin_id));
    fs::create_dir_all(&dir).map_err(|e| format!("create_dir_all {}: {e}", dir.display()))?;
    Ok(dir)
}

fn index_path(dir: &Path) -> PathBuf {
    dir.join("index.json")
}

fn blob_path(dir: &Path, preset_id: &str) -> PathBuf {
    // The id is generated via uuid::Uuid::new_v4() below — already safe
    // as a filename. Still sanitize defensively in case external code
    // ever passes one through.
    dir.join(format!("{}.bin", sanitize(preset_id)))
}

fn read_index(dir: &Path) -> Vec<PresetInfo> {
    match fs::read(index_path(dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn write_index(dir: &Path, list: &[PresetInfo]) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(list).map_err(|e| format!("serialize: {e}"))?;
    fs::write(index_path(dir), bytes).map_err(|e| format!("write index: {e}"))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[tauri::command]
pub fn list_plugin_presets(app: AppHandle, plugin_id: String) -> Result<Vec<PresetInfo>, String> {
    let dir = preset_dir(&app, &plugin_id)?;
    Ok(read_index(&dir))
}

#[tauri::command]
pub fn save_plugin_preset(
    app: AppHandle,
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    plugin_id: String,
    name: String,
) -> Result<PresetInfo, String> {
    // Capture the slot's current state via the engine's existing
    // snapshot mechanism. This blocks the UI thread for one audio
    // block while the audio thread harvests get_state — same path
    // project-save uses.
    let map = state
        .engine
        .lock()
        .snapshot_plugin_states(std::time::Duration::from_millis(500))
        .ok_or_else(|| "engine did not service snapshot request".to_string())?;
    let bytes = map
        .get(&(track_id.clone(), slot_id.clone()))
        .ok_or_else(|| format!("no state for slot {}/{}", track_id, slot_id))?
        .clone();

    let dir = preset_dir(&app, &plugin_id)?;
    // The folder name is the plug-in id with everything awkward replaced,
    // which cannot be turned back into the id. The sidecar keeps the real
    // one so the browser can list every plug-in's presets together.
    write_owner(&dir, &plugin_id);
    let preset_id = uuid::Uuid::new_v4().to_string();
    fs::write(blob_path(&dir, &preset_id), &bytes).map_err(|e| format!("write blob: {e}"))?;

    let info = PresetInfo {
        id: preset_id.clone(),
        name: name.trim().to_string(),
        created_at: unix_now(),
    };

    let mut list = read_index(&dir);
    // Insert at the front so most-recently-created is the natural
    // "next" target when the user hits the > arrow on a fresh slot.
    list.insert(0, info.clone());
    write_index(&dir, &list)?;

    Ok(info)
}

#[tauri::command]
pub fn load_plugin_preset(
    app: AppHandle,
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    plugin_id: String,
    preset_id: String,
) -> Result<(), String> {
    let dir = preset_dir(&app, &plugin_id)?;
    let bytes = fs::read(blob_path(&dir, &preset_id)).map_err(|e| format!("read blob: {e}"))?;
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

#[tauri::command]
pub fn delete_plugin_preset(
    app: AppHandle,
    plugin_id: String,
    preset_id: String,
) -> Result<(), String> {
    let dir = preset_dir(&app, &plugin_id)?;
    // Best-effort delete; missing blob is fine, we still want to prune
    // the index entry.
    let _ = fs::remove_file(blob_path(&dir, &preset_id));
    let mut list = read_index(&dir);
    list.retain(|p| p.id != preset_id);
    write_index(&dir, &list)?;
    Ok(())
}

#[tauri::command]
pub fn rename_plugin_preset(
    app: AppHandle,
    plugin_id: String,
    preset_id: String,
    new_name: String,
) -> Result<(), String> {
    let dir = preset_dir(&app, &plugin_id)?;
    let mut list = read_index(&dir);
    let mut hit = false;
    for p in list.iter_mut() {
        if p.id == preset_id {
            p.name = new_name.trim().to_string();
            hit = true;
            break;
        }
    }
    if !hit {
        return Err(format!("preset {preset_id} not found"));
    }
    write_index(&dir, &list)
}

/// The real plug-in id, written beside the index because the folder name
/// is a one-way flattening of it.
fn owner_path(dir: &Path) -> PathBuf {
    dir.join("plugin.json")
}

fn write_owner(dir: &Path, plugin_id: &str) {
    let body = serde_json::json!({ "pluginId": plugin_id });
    if let Ok(bytes) = serde_json::to_vec_pretty(&body) {
        let _ = fs::write(owner_path(dir), bytes);
    }
}

fn read_owner(dir: &Path) -> Option<String> {
    let bytes = fs::read(owner_path(dir)).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("pluginId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// One plug-in's saved presets, for the browser that shows them all.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetBank {
    pub plugin_id: String,
    pub presets: Vec<PresetInfo>,
}

/// Every saved preset, grouped by the plug-in it belongs to.
///
/// Presets used to be reachable only from the slot that made them, so a
/// patch saved on Insert 3 was invisible while working on Insert 11. This
/// walks the presets folder instead, and a folder from an older build
/// without the sidecar is listed under its flattened name rather than
/// dropped.
#[tauri::command]
pub fn list_all_presets(app: AppHandle) -> Result<Vec<PresetBank>, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?
        .join("hardwave")
        .join("presets");
    let Ok(entries) = fs::read_dir(&base) else {
        return Ok(Vec::new());
    };
    let mut banks: Vec<PresetBank> = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let presets = read_index(&dir);
        if presets.is_empty() {
            continue;
        }
        let plugin_id = read_owner(&dir).unwrap_or_else(|| {
            dir.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        });
        banks.push(PresetBank { plugin_id, presets });
    }
    banks.sort_by_key(|b| b.plugin_id.to_lowercase());
    Ok(banks)
}

/// The presets that ship inside a plug-in.
///
/// The browser could only ever show what the user had saved, because
/// the host never asked a plug-in what it carries. A VST3 publishes a
/// program list; a CLAP publishes presets through a factory the host
/// does not read yet, so a CLAP answers with an empty list.
///
/// Read from a throwaway instance, the same way the parameter sheet
/// reads names, so nothing on the audio thread is disturbed.
#[tauri::command]
pub fn list_factory_presets(
    state: State<AppState>,
    plugin_id: String,
) -> Result<Vec<String>, String> {
    let descriptor = {
        let engine = state.engine.lock();
        let scanner = engine.plugin_scanner.lock();
        scanner
            .plugins()
            .iter()
            .find(|d| d.id == plugin_id)
            .cloned()
            .ok_or_else(|| format!("No plug-in with id {plugin_id}"))?
    };
    let plugin = crate::commands::plugins::load_hosted(&descriptor)?;
    Ok(plugin.factory_presets())
}

/// Play one of a plug-in's own presets in a live slot.
#[tauri::command]
pub fn load_factory_preset(
    state: State<AppState>,
    track_id: String,
    slot_id: String,
    index: usize,
) -> Result<(), String> {
    let cmd = InsertCommand::LoadFactoryPreset {
        track_id,
        slot_id,
        index,
    };
    state
        .engine
        .lock()
        .try_send_insert_command(cmd)
        .map_err(|_| "insert command queue full or engine not started".to_string())
}
