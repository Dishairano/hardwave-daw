use crate::AppState;
use serde::Serialize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};

const AUTOSAVE_DIR: &str = "autosaves";
const CRASH_MARKER: &str = "session.alive";
const MAX_AUTOSAVES: usize = 3;

fn autosave_root(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("cache dir: {}", e))?;
    let dir = base.join(AUTOSAVE_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {:?}: {}", dir, e))?;
    Ok(dir)
}

#[derive(Serialize)]
pub struct AutosaveInfo {
    path: String,
    modified_unix: u64,
}

/// Write an auto-save into the cache dir and prune older snapshots beyond MAX_AUTOSAVES.
#[tauri::command]
pub fn autosave_save(app: AppHandle, state: State<AppState>) -> Result<String, String> {
    let dir = autosave_root(&app)?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = format!("autosave-{}.hwp", ts);
    let path = dir.join(&name);

    {
        let engine = state.engine.lock();
        let project = engine.project.lock();
        project
            .save(&path)
            .map_err(|e| format!("save autosave: {}", e))?;
    }

    // Prune oldest beyond MAX_AUTOSAVES.
    let mut entries: Vec<(PathBuf, u64)> = std::fs::read_dir(&dir)
        .map_err(|e| format!("read {:?}: {}", dir, e))?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let ext_ok = p.extension().and_then(|s| s.to_str()) == Some("hwp");
            let name_ok = p
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("autosave-"));
            if !ext_ok || !name_ok {
                return None;
            }
            let mtime = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            Some((p, mtime))
        })
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    for (old_path, _) in entries.into_iter().skip(MAX_AUTOSAVES) {
        let _ = std::fs::remove_file(old_path);
    }

    Ok(path.to_string_lossy().into_owned())
}

/// Return the most recent auto-save, if any.
#[tauri::command]
pub fn autosave_latest(app: AppHandle) -> Result<Option<AutosaveInfo>, String> {
    let dir = autosave_root(&app)?;
    let mut newest: Option<AutosaveInfo> = None;
    let rd = match std::fs::read_dir(&dir) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    for e in rd.flatten() {
        let p = e.path();
        let name_ok = p
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.starts_with("autosave-") && s.ends_with(".hwp"));
        if !name_ok {
            continue;
        }
        let mtime = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        match &newest {
            Some(n) if n.modified_unix >= mtime => {}
            _ => {
                newest = Some(AutosaveInfo {
                    path: p.to_string_lossy().into_owned(),
                    modified_unix: mtime,
                });
            }
        }
    }
    Ok(newest)
}

/// Remove all auto-save files — call this after a successful user-initiated save.
#[tauri::command]
pub fn autosave_clear(app: AppHandle) -> Result<(), String> {
    let dir = autosave_root(&app)?;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            let is_autosave = p
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("autosave-") && s.ends_with(".hwp"));
            if is_autosave {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    Ok(())
}

/// Create a "session alive" marker. If the marker is still present on next startup,
/// the app exited uncleanly and a crash recovery prompt should be offered.
#[tauri::command]
pub fn autosave_mark_alive(app: AppHandle) -> Result<(), String> {
    let dir = autosave_root(&app)?;
    let marker = dir.join(CRASH_MARKER);
    std::fs::write(&marker, b"1").map_err(|e| format!("write {:?}: {}", marker, e))?;
    Ok(())
}

/// Remove the "session alive" marker on clean shutdown.
#[tauri::command]
pub fn autosave_clear_alive(app: AppHandle) -> Result<(), String> {
    let dir = autosave_root(&app)?;
    let marker = dir.join(CRASH_MARKER);
    let _ = std::fs::remove_file(marker);
    Ok(())
}

/// Returns true if a previous session crashed (alive marker is still present).
/// Also clears the marker so a subsequent query returns false.
#[tauri::command]
pub fn autosave_detect_crash(app: AppHandle) -> Result<bool, String> {
    let dir = autosave_root(&app)?;
    let marker = dir.join(CRASH_MARKER);
    let crashed = marker.exists();
    if crashed {
        let _ = std::fs::remove_file(marker);
    }
    Ok(crashed)
}
