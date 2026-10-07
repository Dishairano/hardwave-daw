//! Separating a clip into drums, bass, vocals and the rest (Pro).
//!
//! The clip's audio file goes to the stems service on our server, and
//! the four parts come back at the file's own sample rate. Each part
//! goes on a track of its own right under the clip's track, at the same
//! place and trimmed the same way, so pressing play sounds the same as
//! before; the original clip is muted rather than removed. The parts
//! are kept in a folder beside the song.

use crate::stems_client::{self, Client, STEMS};
use crate::AppState;
use hardwave_engine::DawEngine;
use hardwave_project::clip::{AudioClip, ClipContent};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

static STOP: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StemsProgress {
    /// sending, waiting, separating or receiving.
    pub stage: &'static str,
    /// Songs ahead in the line while waiting.
    pub ahead: usize,
    /// 0..1 within the stage, where the stage has a measure.
    pub progress: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StemsResult {
    pub tracks: Vec<String>,
    pub folder: String,
}

fn title(stem: &str) -> &'static str {
    match stem {
        "drums" => "Drums",
        "bass" => "Bass",
        "vocals" => "Vocals",
        _ => "Other",
    }
}

/// Where the parts are kept: beside the song when it has been saved,
/// otherwise under Documents.
fn stems_folder(project_path: Option<&str>) -> PathBuf {
    match project_path.map(Path::new) {
        Some(p) if p.parent().is_some() => {
            let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Song");
            p.parent().unwrap().join(format!("{name} Stems"))
        }
        _ => dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(std::env::temp_dir)
            .join("Hardwave")
            .join("Stems"),
    }
}

/// A name that is safe as a file name on every system.
fn file_safe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if "\\/:*?\"<>|".contains(c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Clip".into()
    } else {
        trimmed.chars().take(80).collect()
    }
}

/// The clip to separate and the file behind it.
fn find_clip(
    engine: &DawEngine,
    track_id: &str,
    clip_id: &str,
) -> Result<(AudioClip, PathBuf), String> {
    let project = engine.project.lock();
    let track = project
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .ok_or("that track is gone")?;
    let clip = track
        .clips
        .iter()
        .find_map(|p| match &p.content {
            ClipContent::Audio(a) if a.id == clip_id => Some(a.clone()),
            _ => None,
        })
        .ok_or("only an audio clip can be separated")?;
    let file = PathBuf::from(&clip.source_file);
    if clip.source_file.is_empty() || !file.is_file() {
        return Err("the audio file behind this clip is missing; relink it first".into());
    }
    Ok((clip, file))
}

/// Put each part on its own track under the clip's track, where the
/// clip is and trimmed as it is, and mute the clip. One undo step.
pub(crate) fn lay_out_stems(
    engine: &DawEngine,
    track_id: &str,
    clip_id: &str,
    parts: &[(&str, PathBuf)],
) -> Result<Vec<String>, String> {
    engine.snapshot_before_mutation();
    let mut loaded = Vec::new();
    for (stem, path) in parts {
        let (source_id, info) = engine.load_audio_file(path)?;
        loaded.push((*stem, path.clone(), source_id, info.total_frames));
    }

    let mut ids = Vec::new();
    {
        let mut project = engine.project.lock();
        let at = project
            .tracks
            .iter()
            .position(|t| t.id == track_id)
            .ok_or("that track is gone")?;
        let color = project.tracks[at].color.clone();
        let (placement, clip) = project.tracks[at]
            .clips
            .iter_mut()
            .find_map(|p| match &mut p.content {
                ClipContent::Audio(a) if a.id == clip_id => {
                    a.muted = true;
                    let clip = a.clone();
                    Some((p.clone(), clip))
                }
                _ => None,
            })
            .ok_or("that clip is gone")?;

        for (offset, (stem, path, source_id, frames)) in loaded.into_iter().enumerate() {
            let id = project.add_audio_track(format!("{} {}", clip.name, title(stem)));
            let index = project
                .tracks
                .iter()
                .position(|t| t.id == id)
                .expect("just added");
            let mut track = project.tracks.remove(index);
            track.color = color.clone();
            let mut part = clip.clone();
            part.id = uuid::Uuid::new_v4().to_string();
            part.name = format!("{} {}", clip.name, title(stem));
            part.source_path = source_id;
            part.source_hash = crate::commands::audio::hash_source_file(&path);
            part.source_file = path.to_string_lossy().into_owned();
            part.muted = false;
            // The part is the song's length at the song's rate; a frame
            // either way from resampling is clamped rather than read past.
            part.source_end = part.source_end.min(frames);
            part.source_start = part.source_start.min(part.source_end);
            let mut placed = placement.clone();
            placed.track_id = id.clone();
            placed.content = ClipContent::Audio(part);
            track.clips.push(placed);
            project.tracks.insert(at + 1 + offset, track);
            ids.push(id);
        }
    }
    engine.sync_track_meters();
    engine.rebuild_graph();
    Ok(ids)
}

/// Send a clip's audio to be separated and lay the parts out under it.
#[tauri::command]
pub async fn separate_stems(
    app: AppHandle,
    track_id: String,
    clip_id: String,
    project_path: Option<String>,
) -> Result<StemsResult, String> {
    STOP.store(false, Ordering::SeqCst);
    let token = crate::collab::load_token().ok_or_else(|| {
        "sign in first. Opening any Hardwave plug-in and signing in there is enough: \
         every Hardwave program on this machine shares one sign-in."
            .to_string()
    })?;
    let (clip, file) = {
        let state = app.state::<AppState>();
        let engine = state.engine.lock();
        find_clip(&engine, &track_id, &clip_id)?
    };
    let progress = |stage: &'static str, ahead: usize, progress: f32| {
        let _ = app.emit(
            "stems-progress",
            StemsProgress {
                stage,
                ahead,
                progress,
            },
        );
    };

    progress("sending", 0, 0.0);
    let song = tokio::fs::read(&file)
        .await
        .map_err(|e| format!("could not read {}: {e}", file.display()))?;
    let file_name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("song")
        .to_string();
    let client = Client::new(token);
    let job = client.submit(&file_name, song).await?;

    loop {
        if STOP.load(Ordering::SeqCst) {
            client.cancel(&job.id).await;
            return Err("stopped".into());
        }
        let now = client.status(&job.id).await?;
        match now.state.as_str() {
            "queued" => progress("waiting", now.ahead, 0.0),
            "running" => progress("separating", 0, now.progress),
            "done" => break,
            _ => {
                return Err(now
                    .error
                    .unwrap_or_else(|| "the separation failed on the server".into()))
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    let folder = stems_folder(project_path.as_deref());
    tokio::fs::create_dir_all(&folder)
        .await
        .map_err(|e| format!("could not make {}: {e}", folder.display()))?;
    let mut parts = Vec::new();
    for (i, stem) in STEMS.iter().enumerate() {
        progress("receiving", 0, i as f32 / STEMS.len() as f32);
        let to = folder.join(format!("{} - {}.flac", file_safe(&clip.name), title(stem)));
        client.download(&job.id, stem, &to).await?;
        parts.push((*stem, to));
    }

    let state = app.state::<AppState>();
    let engine = state.engine.lock();
    let tracks = lay_out_stems(&engine, &track_id, &clip_id, &parts)?;
    Ok(StemsResult {
        tracks,
        folder: folder.to_string_lossy().into_owned(),
    })
}

/// Stop a separation: out of the line, or stopped on the server.
#[tauri::command]
pub fn stop_stems() {
    STOP.store(true, Ordering::SeqCst);
}

/// Where the stems service is, for the window's "what is this" text.
#[tauri::command]
pub fn stems_service() -> String {
    stems_client::base_url()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_wav() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_assets/pink_noise_-12dbfs_10s.wav")
    }

    #[test]
    fn the_parts_land_under_the_clip_trimmed_like_it_and_the_clip_goes_quiet() {
        let engine = DawEngine::new();
        let (track_id, clip_id) = {
            let (source_id, info) = engine.load_audio_file(&test_wav()).unwrap();
            let mut project = engine.project.lock();
            let track_id = project.add_audio_track("Loop".into());
            let after = project.add_audio_track("After".into());
            let clip = AudioClip {
                id: "clip-1".into(),
                name: "Raw Loop".into(),
                source_path: source_id,
                source_hash: String::new(),
                source_start: 1000,
                source_end: info.total_frames - 500,
                gain_db: -3.0,
                fade_in_ticks: 10,
                fade_out_ticks: 20,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                warp_markers: Vec::new(),
                fade_in_curve: Default::default(),
                fade_out_curve: Default::default(),
                source_file: test_wav().to_string_lossy().into_owned(),
            };
            project.track_mut(&track_id).unwrap().clips.push(
                hardwave_project::clip::ClipPlacement {
                    content: ClipContent::Audio(clip),
                    track_id: track_id.clone(),
                    position_ticks: 3840,
                    length_ticks: 7680,
                    lane: 0,
                },
            );
            let _ = after;
            (track_id, "clip-1".to_string())
        };

        let (clip, _) = find_clip(&engine, &track_id, &clip_id).unwrap();
        // The service's parts are the song's length; the test file
        // stands in for each.
        let parts: Vec<(&str, PathBuf)> = STEMS.iter().map(|s| (*s, test_wav())).collect();
        let ids = lay_out_stems(&engine, &track_id, &clip_id, &parts).unwrap();
        assert_eq!(ids.len(), 4);

        let project = engine.project.lock();
        let names: Vec<&str> = project.tracks.iter().map(|t| t.name.as_str()).collect();
        let at = names.iter().position(|n| *n == "Loop").unwrap();
        assert_eq!(
            &names[at..at + 6],
            &[
                "Loop",
                "Raw Loop Drums",
                "Raw Loop Bass",
                "Raw Loop Other",
                "Raw Loop Vocals",
                "After"
            ],
            "the parts sit right under the clip's track, before anything that was there"
        );
        for id in &ids {
            let track = project.tracks.iter().find(|t| &t.id == id).unwrap();
            let placed = &track.clips[0];
            assert_eq!((placed.position_ticks, placed.length_ticks), (3840, 7680));
            let ClipContent::Audio(part) = &placed.content else {
                panic!("audio")
            };
            assert_eq!(
                (part.source_start, part.source_end),
                (clip.source_start, clip.source_end)
            );
            assert_eq!(part.gain_db, -3.0);
            assert_eq!((part.fade_in_ticks, part.fade_out_ticks), (10, 20));
            assert!(!part.muted);
        }
        let original = project.tracks[at]
            .clips
            .iter()
            .find_map(|p| match &p.content {
                ClipContent::Audio(a) => Some(a.clone()),
                _ => None,
            });
        assert!(original.unwrap().muted, "the clip is muted, not removed");
    }

    #[test]
    fn a_missing_file_is_said_before_anything_is_sent() {
        let engine = DawEngine::new();
        let track_id = engine.project.lock().add_midi_track("Lead".into());
        assert_eq!(
            find_clip(&engine, &track_id, "nope").unwrap_err(),
            "only an audio clip can be separated"
        );
    }

    #[test]
    fn the_parts_folder_sits_beside_the_song() {
        let beside = stems_folder(Some("/music/Raw Drop/Raw Drop.hwp"));
        assert_eq!(beside, PathBuf::from("/music/Raw Drop/Raw Drop Stems"));
        assert!(stems_folder(None).ends_with("Hardwave/Stems"));
        assert_eq!(file_safe("Kick: \"final\" v2/3"), "Kick   final  v2 3");
        assert_eq!(file_safe("  "), "Clip");
    }
}
