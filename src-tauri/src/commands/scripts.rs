//! The script library, and applying what a script asked for.
//!
//! A script collects commands, and they are applied here in one go,
//! under one undo step: a script that writes forty notes is one
//! ctrl-Z, not forty.

use crate::AppState;
use hardwave_project::scripting_api::ScriptCommand;
use serde::Serialize;
use tauri::State;

/// One script on disk.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptFile {
    pub name: String,
    pub body: String,
}

/// What running one did.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptResult {
    /// What it printed, in order.
    pub output: Vec<String>,
    /// How many commands it asked for, and how many went through. A
    /// command naming a track that is not there is skipped rather than
    /// failing the run, and the difference is what says so.
    pub asked: usize,
    pub applied: usize,
}

/// Where scripts live: next to the project settings, as plain text, so
/// they can be edited outside the DAW and shared as files.
fn scripts_dir() -> std::path::PathBuf {
    let base = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    base.join("hardwave").join("daw").join("scripts")
}

/// A name that cannot escape the scripts folder.
fn safe_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ' ')
    {
        return None;
    }
    Some(trimmed.to_string())
}

#[tauri::command]
pub fn list_scripts() -> Vec<ScriptFile> {
    let dir = scripts_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<ScriptFile> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "rhai"))
        .filter_map(|e| {
            let name = e.path().file_stem()?.to_str()?.to_string();
            let body = std::fs::read_to_string(e.path()).ok()?;
            Some(ScriptFile { name, body })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[tauri::command]
pub fn save_script(name: String, body: String) -> Result<(), String> {
    let name = safe_name(&name)
        .ok_or_else(|| "a script's name is letters, numbers, spaces, - and _".to_string())?;
    let dir = scripts_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make the scripts folder: {e}"))?;
    std::fs::write(dir.join(format!("{name}.rhai")), body)
        .map_err(|e| format!("could not save the script: {e}"))
}

#[tauri::command]
pub fn delete_script(name: String) -> Result<(), String> {
    let name = safe_name(&name).ok_or_else(|| "that is not a script name".to_string())?;
    let path = scripts_dir().join(format!("{name}.rhai"));
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(path).map_err(|e| format!("could not delete the script: {e}"))
}

/// Check a script without changing anything.
///
/// The same run, with the commands thrown away: it is how the editor
/// can say "this does not parse" before the project is touched.
#[tauri::command]
pub fn check_script(body: String) -> Result<ScriptResult, String> {
    let run = crate::scripting::run(&body)?;
    Ok(ScriptResult {
        output: run.output,
        asked: run.commands.len(),
        applied: 0,
    })
}

/// Run a script and apply what it asked for.
#[tauri::command]
pub fn run_script(state: State<AppState>, body: String) -> Result<ScriptResult, String> {
    let run = crate::scripting::run(&body)?;
    if run.commands.is_empty() {
        return Ok(ScriptResult {
            output: run.output,
            asked: 0,
            applied: 0,
        });
    }

    // One snapshot for the whole script, so undoing it is one step.
    state.engine.lock().snapshot_before_mutation();
    let mut applied = 0usize;
    let mut touched_project = false;
    for command in &run.commands {
        if apply(&state, command, &mut touched_project) {
            applied += 1;
        }
    }
    if touched_project {
        state.engine.lock().rebuild_graph();
    }

    Ok(ScriptResult {
        output: run.output,
        asked: run.commands.len(),
        applied,
    })
}

/// A clip's id, whichever kind it is. A placement carries its content
/// rather than an id of its own.
fn clip_identity(clip: &hardwave_project::clip::ClipPlacement) -> String {
    match &clip.content {
        hardwave_project::clip::ClipContent::Midi(mc) => mc.id.clone(),
        hardwave_project::clip::ClipContent::Audio(ac) => ac.id.clone(),
    }
}

/// Apply one command. Returns whether it found what it was aimed at: a
/// script naming a track that is not there should say so at the end,
/// not stop half way through and leave the project in between.
fn apply(state: &State<AppState>, command: &ScriptCommand, touched_project: &mut bool) -> bool {
    use hardwave_engine::transport::TransportCommand;
    use std::sync::atomic::Ordering;

    let engine = state.engine.lock();
    match command {
        ScriptCommand::TransportPlay => {
            engine.send_command(TransportCommand::Play);
            true
        }
        ScriptCommand::TransportStop => {
            engine.send_command(TransportCommand::Stop);
            true
        }
        ScriptCommand::TransportSeek { tick } => {
            let sample_rate = engine.current_sample_rate() as f64;
            let bpm = engine.transport.bpm.load(Ordering::Relaxed);
            let beats = *tick as f64 / hardwave_midi::PPQ as f64;
            engine
                .transport
                .set_position((beats * 60.0 / bpm * sample_rate) as u64);
            true
        }
        ScriptCommand::SetMasterVolume { db } => {
            engine
                .transport
                .master_volume_db
                .store((*db as f64).clamp(-100.0, 12.0), Ordering::Relaxed);
            true
        }
        ScriptCommand::SetTrackVolume { track_id, db } => {
            let mut project = engine.project.lock();
            match project.track_mut(track_id) {
                Some(track) => {
                    track.volume_db = (*db as f64).clamp(-100.0, 12.0);
                    *touched_project = true;
                    true
                }
                None => false,
            }
        }
        ScriptCommand::SetTrackPan { track_id, pan } => {
            let mut project = engine.project.lock();
            match project.track_mut(track_id) {
                Some(track) => {
                    track.pan = (*pan as f64).clamp(-1.0, 1.0);
                    *touched_project = true;
                    true
                }
                None => false,
            }
        }
        ScriptCommand::SetTrackMuted { track_id, muted } => {
            let mut project = engine.project.lock();
            match project.track_mut(track_id) {
                Some(track) => {
                    track.muted = *muted;
                    *touched_project = true;
                    true
                }
                None => false,
            }
        }
        ScriptCommand::InsertNote {
            clip_id,
            tick,
            pitch,
            velocity,
            length_ticks,
        } => {
            let mut project = engine.project.lock();
            for track in project.tracks.iter_mut() {
                for clip in track.clips.iter_mut() {
                    if let hardwave_project::clip::ClipContent::Midi(mc) = &mut clip.content {
                        if mc.id == *clip_id {
                            mc.clip.notes.push(hardwave_midi::MidiNote {
                                start_tick: *tick,
                                duration_ticks: *length_ticks,
                                pitch: *pitch,
                                velocity: *velocity as f32 / 127.0,
                                channel: 0,
                                muted: false,
                                ..Default::default()
                            });
                            *touched_project = true;
                            return true;
                        }
                    }
                }
            }
            false
        }
        ScriptCommand::DeleteNote {
            clip_id,
            tick,
            pitch,
        } => {
            let mut project = engine.project.lock();
            for track in project.tracks.iter_mut() {
                for clip in track.clips.iter_mut() {
                    if let hardwave_project::clip::ClipContent::Midi(mc) = &mut clip.content {
                        if mc.id == *clip_id {
                            let before = mc.clip.notes.len();
                            mc.clip
                                .notes
                                .retain(|n| !(n.start_tick == *tick && n.pitch == *pitch));
                            let removed = mc.clip.notes.len() != before;
                            *touched_project |= removed;
                            return removed;
                        }
                    }
                }
            }
            false
        }
        ScriptCommand::MoveClip {
            clip_id,
            new_start_tick,
        } => {
            let mut project = engine.project.lock();
            for track in project.tracks.iter_mut() {
                for clip in track.clips.iter_mut() {
                    if clip_identity(clip) == *clip_id {
                        clip.position_ticks = *new_start_tick;
                        *touched_project = true;
                        return true;
                    }
                }
            }
            false
        }
        ScriptCommand::DeleteClip { clip_id } => {
            let mut project = engine.project.lock();
            let mut removed = false;
            for track in project.tracks.iter_mut() {
                let before = track.clips.len();
                track.clips.retain(|c| clip_identity(c) != *clip_id);
                removed |= track.clips.len() != before;
            }
            *touched_project |= removed;
            removed
        }
        // A script asking for a panel is answered by the UI, which
        // knows what a panel is; here it is counted and let through.
        ScriptCommand::OpenPanel { .. } => true,
        // Running a script from a script needs the library, which the
        // UI holds. Not yet: a script that calls itself is a loop with
        // no guard, and this one has to be a deliberate piece of work.
        ScriptCommand::RunScript { .. } => false,
        ScriptCommand::CreateClip { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_cannot_climb_out_of_the_folder() {
        assert_eq!(safe_name("../../etc/passwd"), None);
        assert_eq!(safe_name("drums/one"), None);
        assert_eq!(safe_name(""), None);
        assert_eq!(safe_name("  "), None);
        assert_eq!(safe_name("double kick"), Some("double kick".to_string()));
        assert_eq!(safe_name(" trim_me "), Some("trim_me".to_string()));
    }
}
