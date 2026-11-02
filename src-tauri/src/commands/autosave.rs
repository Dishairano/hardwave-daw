use crate::AppState;
use hardwave_project::Project;
use serde::Serialize;
use std::path::{Path, PathBuf};
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

/// The song as an auto-save writes it: what Save would write, with every
/// sample the song names relative to its own folder (`song_dir`) written
/// as a full path.
///
/// The auto-save used to be the bare song model. Plug-in settings and MIDI
/// Learn only reach the model when the song is saved, so a song recovered
/// after a crash came back with every synth and effect as it was at the
/// last Save. And an auto-save lives in the app's cache folder, where a
/// collected song's "Song Samples/kick.wav" points at nothing, so its
/// samples all came back missing.
fn autosave_copy(
    project: &Project,
    live: crate::commands::project::LiveState,
    song_dir: Option<&Path>,
) -> Project {
    let mut copy = project.clone();
    live.write_into(&mut copy);
    if let Some(dir) = song_dir {
        copy.anchor_sample_files(dir);
    }
    copy
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

    let live = crate::commands::project::live_state(&state);
    let copy = {
        let engine = state.engine.lock();
        let project = engine.project.lock();
        autosave_copy(&project, live, engine.project_dir().as_deref())
    };
    copy.save(&path)
        .map_err(|e| format!("save autosave: {}", e))?;

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
    entries.sort_by_key(|e| std::cmp::Reverse(e.1));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::project::LiveState;
    use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement};

    fn collected_clip(file: &str) -> ClipPlacement {
        ClipPlacement {
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
                source_file: file.into(),
                treatment: Default::default(),
                tuned_notes: Vec::new(),
            }),
            track_id: "insert-001".into(),
            position_ticks: 0,
            length_ticks: 3840,
            lane: 0,
            color: None,
        }
    }

    #[test]
    fn an_autosave_holds_the_plug_ins_as_they_play_and_finds_collected_samples() {
        let song_dir = std::env::temp_dir().join("Anthem");
        let mut song = Project::default();
        // The synth as it was at the last Save.
        song.set_plugin_state("slot-synth", "unknown", vec![1]);
        song.track_mut("insert-001")
            .unwrap()
            .clips
            .push(collected_clip("Anthem Samples/kick.wav"));

        // Turned since: what the audio thread reports now.
        let live = LiveState::for_test(
            [(
                ("insert-001".to_string(), "slot-synth".to_string()),
                vec![9],
            )]
            .into_iter()
            .collect(),
            Some("[]".into()),
        );
        let copy = autosave_copy(&song, live, Some(&song_dir));

        assert_eq!(copy.plugin_state("slot-synth").unwrap().chunk, vec![9]);
        assert_eq!(copy.midi_mappings.as_deref(), Some("[]"));
        let ClipContent::Audio(kick) = &copy.track("insert-001").unwrap().clips[0].content else {
            panic!("an audio clip");
        };
        assert_eq!(
            kick.source_file,
            song_dir
                .join("Anthem Samples/kick.wav")
                .to_string_lossy()
                .into_owned()
        );
        assert_eq!(kick.source_path, "9f2c", "the pool id is kept");
        // The open song itself is left alone: its folder is still its own.
        assert_eq!(song.plugin_state("slot-synth").unwrap().chunk, vec![1]);
    }
}
