//! Track and song templates, kept as files in the user's data folder.
//!
//! Templates used to live in the window's storage and held a track's name,
//! colour, fader and pan only: a vocal chain saved as a template came back
//! as an empty track. A track template now keeps the plug-ins with their
//! settings and the bus and sends it fed; a song template is a whole song
//! file, plug-ins, routing and all, opened as a new untitled song.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::AppState;
use hardwave_project::templates::TrackTemplate;
use hardwave_project::Project;

const TRACK_EXT: &str = "hwtrack";
const SONG_EXT: &str = "hwp";

fn templates_dir(kind: &str) -> Result<PathBuf, String> {
    let base = dirs::data_dir()
        .ok_or_else(|| "No data folder on this system for templates".to_string())?;
    let dir = base.join("Hardwave").join("Templates").join(kind);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not make the templates folder: {e}"))?;
    Ok(dir)
}

/// A template's file name from the name it was given. Names are kept to
/// what a file name can hold on every system, so a name can never point
/// outside the templates folder.
fn file_for(dir: &Path, name: &str, ext: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the template a name".into());
    }
    if name.chars().count() > 60 {
        return Err("Keep the name to 60 characters".into());
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '(' | ')' | '&' | '+' | ','))
        || name.starts_with('.')
    {
        return Err("Use letters, numbers, spaces and - _ ( ) & + , in a template name".into());
    }
    // Windows reads these as devices whatever follows them: "CON.hwtrack"
    // is the console, not a file.
    let stem = name.split(' ').next().unwrap_or(name).to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if device && name.trim_end().len() == stem.len() {
        return Err("That name is kept by Windows; pick another".into());
    }
    Ok(dir.join(format!("{name}.{ext}")))
}

fn names_in(dir: &Path, ext: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    let path = e.path();
                    (path.extension().and_then(|x| x.to_str()) == Some(ext))
                        .then(|| path.file_stem()?.to_str().map(str::to_string))
                        .flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Every plug-in's settings as it plays now, by slot id.
fn live_states(state: &State<AppState>) -> HashMap<String, Vec<u8>> {
    state
        .engine
        .lock()
        .snapshot_plugin_states(std::time::Duration::from_millis(500))
        .unwrap_or_default()
        .into_iter()
        .map(|((_, slot), bytes)| (slot, bytes))
        .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackTemplateEntry {
    pub name: String,
    /// "Audio" or "Midi".
    pub kind: String,
    pub plugins: usize,
}

/// Keep a track as a template, with its plug-ins and their settings as
/// they play now. A template of the same name is replaced.
#[tauri::command]
pub fn save_track_template(
    state: State<AppState>,
    track_id: String,
    name: String,
) -> Result<(), String> {
    let path = file_for(&templates_dir("Tracks")?, &name, TRACK_EXT)?;
    let live = live_states(&state);
    let template = {
        let engine = state.engine.lock();
        let project = engine.project.lock();
        project
            .track_template(&track_id, &live)
            .ok_or_else(|| "That track cannot be kept as a template".to_string())?
    };
    std::fs::write(&path, template.to_bytes())
        .map_err(|e| format!("Could not write the template: {e}"))
}

#[tauri::command]
pub fn list_track_templates() -> Vec<TrackTemplateEntry> {
    let Ok(dir) = templates_dir("Tracks") else {
        return Vec::new();
    };
    names_in(&dir, TRACK_EXT)
        .into_iter()
        .filter_map(|name| {
            let bytes = std::fs::read(dir.join(format!("{name}.{TRACK_EXT}"))).ok()?;
            let t = TrackTemplate::from_bytes(&bytes)?;
            Some(TrackTemplateEntry {
                kind: format!("{:?}", t.track.kind),
                plugins: t.track.inserts.len(),
                name,
            })
        })
        .collect()
}

/// Add a track made from a template, after `after_track_id` or last, with
/// its plug-ins loaded. Returns the new track's id.
#[tauri::command]
pub fn add_track_from_template(
    app: AppHandle,
    name: String,
    after_track_id: Option<String>,
) -> Result<String, String> {
    let path = file_for(&templates_dir("Tracks")?, &name, TRACK_EXT)?;
    let bytes = std::fs::read(&path).map_err(|_| format!("No track template called {name}"))?;
    let template = TrackTemplate::from_bytes(&bytes)
        .ok_or_else(|| format!("The template {name} could not be read"))?;
    let state = app.state::<AppState>();
    state.engine.lock().snapshot_before_mutation();
    let id = {
        let engine = state.engine.lock();
        let id = engine
            .project
            .lock()
            .add_track_from_template(&template, after_track_id.as_deref());
        engine.sync_track_meters();
        engine.rebuild_graph();
        id
    };
    crate::commands::plugins::hydrate_track_chain(&state, &id)?;
    crate::commands::plugins::tell_windows_track_changed(&app, &id);
    Ok(id)
}

#[tauri::command]
pub fn delete_track_template(name: String) -> Result<(), String> {
    let path = file_for(&templates_dir("Tracks")?, &name, TRACK_EXT)?;
    std::fs::remove_file(&path).map_err(|_| format!("No track template called {name}"))
}

/// Keep the open song as a template: the whole song, with every plug-in's
/// settings as it plays now. The open song keeps its own file and folder.
/// Samples are written by their full path, so the template finds them from
/// its own folder.
#[tauri::command]
pub fn save_song_template(state: State<AppState>, name: String) -> Result<(), String> {
    let path = file_for(&templates_dir("Songs")?, &name, SONG_EXT)?;
    let live = state
        .engine
        .lock()
        .snapshot_plugin_states(std::time::Duration::from_millis(500))
        .unwrap_or_default();
    let engine = state.engine.lock();
    let project_dir = engine.project_dir();
    let mut copy: Project = engine.project.lock().clone();
    drop(engine);
    copy.capture_active_arrangement();
    for ((_, slot_id), bytes) in live {
        copy.set_plugin_state(slot_id, "unknown", bytes);
    }
    if let Some(dir) = project_dir {
        absolute_sources(&mut copy, &dir);
    }
    copy.save(&path)
        .map_err(|e| format!("Could not write the template: {e}"))
}

/// A sample collected into the song's own folder is named relative to it;
/// from the templates folder that would point nowhere.
fn absolute_sources(project: &mut Project, dir: &Path) {
    use hardwave_project::clip::ClipContent;
    for track in project.tracks.iter_mut() {
        for clip in track.clips.iter_mut() {
            if let ClipContent::Audio(a) = &mut clip.content {
                if !a.source_path.is_empty() && Path::new(&a.source_path).is_relative() {
                    a.source_path = dir.join(&a.source_path).to_string_lossy().to_string();
                }
            }
        }
    }
}

#[tauri::command]
pub fn list_song_templates() -> Vec<String> {
    templates_dir("Songs")
        .map(|dir| names_in(&dir, SONG_EXT))
        .unwrap_or_default()
}

/// Open a song template as a new, untitled song: saving it asks for a name
/// and folder, so the template itself is never written over.
#[tauri::command]
pub async fn new_song_from_template(app: AppHandle, name: String) -> Result<(), String> {
    let path = file_for(&templates_dir("Songs")?, &name, SONG_EXT)?;
    tauri::async_runtime::spawn_blocking(move || {
        let loaded =
            Project::load(&path).map_err(|_| format!("The template {name} could not be opened"))?;
        crate::commands::project::open_project_blocking(&app, loaded, None, |_, _| {})
    })
    .await
    .map_err(|e| format!("opening stopped: {e}"))?
}

#[tauri::command]
pub fn delete_song_template(name: String) -> Result<(), String> {
    let path = file_for(&templates_dir("Songs")?, &name, SONG_EXT)?;
    std::fs::remove_file(&path).map_err(|_| format!("No song template called {name}"))
}

#[cfg(test)]
mod tests {
    use super::file_for;
    use std::path::Path;

    /// A name can never reach outside the templates folder.
    #[test]
    fn a_template_name_stays_in_its_folder() {
        let dir = Path::new("/t");
        assert!(file_for(dir, "Vocal chain (lead)", "hwtrack").is_ok());
        assert!(file_for(dir, "Console bus", "hwtrack").is_ok());
        assert!(file_for(dir, "COM1 lead", "hwtrack").is_ok());
        for bad in [
            "../x", "a/b", "a\\b", "..", ".hidden", "C:x", "", "   ", "CON", "nul", "COM1", "lpt9",
        ] {
            assert!(file_for(dir, bad, "hwtrack").is_err(), "{bad:?} refused");
        }
    }
}
