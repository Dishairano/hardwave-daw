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
