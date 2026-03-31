use tauri::State;
use crate::AppState;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct ImportedClip {
    track_id: String,
    clip_id: String,
    name: String,
    source_id: String,
    duration_secs: f64,
    sample_rate: u32,
    channels: u16,
    position_ticks: u64,
    length_ticks: u64,
}

/// Import an audio file onto a track at a given position (in ticks).
#[tauri::command]
pub fn import_audio_file(
    state: State<AppState>,
    track_id: String,
    file_path: String,
    position_ticks: Option<u64>,
) -> Result<ImportedClip, String> {
    let path = PathBuf::from(&file_path);
    let file_name = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();

    // Load audio into pool
    let engine = state.engine.lock();
    let (source_id, info) = engine.load_audio_file(&path)?;

    // Calculate clip length in ticks based on duration and current BPM
    let bpm = engine.transport.bpm.load(std::sync::atomic::Ordering::Relaxed);
    let beats = info.duration_secs * bpm / 60.0;
    let length_ticks = (beats * hardwave_midi::PPQ as f64).round() as u64;
    let pos_ticks = position_ticks.unwrap_or(0);

    // Create clip in project
    let clip_id = uuid::Uuid::new_v4().to_string();
    let audio_clip = hardwave_project::clip::AudioClip {
        id: clip_id.clone(),
        name: file_name.clone(),
        source_path: source_id.clone(),
        source_hash: String::new(),
        source_start: 0,
        source_end: info.total_frames,
        gain_db: 0.0,
        fade_in_ticks: 0,
        fade_out_ticks: 0,
        muted: false,
    };

    let placement = hardwave_project::clip::ClipPlacement {
        content: hardwave_project::clip::ClipContent::Audio(audio_clip),
        track_id: track_id.clone(),
        position_ticks: pos_ticks,
        length_ticks,
        lane: 0,
    };

    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.clips.push(placement);
        } else {
            return Err(format!("Track not found: {}", track_id));
        }
    }

    // Tell engine to rebuild audio graph with new clip
    engine.rebuild_graph();

    Ok(ImportedClip {
        track_id,
        clip_id,
        name: file_name,
        source_id,
        duration_secs: info.duration_secs,
        sample_rate: info.sample_rate,
        channels: info.channels,
        position_ticks: pos_ticks,
        length_ticks,
    })
}

/// Get clips for a specific track.
#[tauri::command]
pub fn get_track_clips(
    state: State<AppState>,
    track_id: String,
) -> Vec<ClipInfo> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    let track = match project.track(&track_id) {
        Some(t) => t,
        None => return vec![],
    };

    track.clips.iter().filter_map(|clip| {
        match &clip.content {
            hardwave_project::clip::ClipContent::Audio(ac) => {
                Some(ClipInfo {
                    id: ac.id.clone(),
                    name: ac.name.clone(),
                    kind: "audio".into(),
                    position_ticks: clip.position_ticks,
                    length_ticks: clip.length_ticks,
                    muted: ac.muted,
                })
            }
            hardwave_project::clip::ClipContent::Midi(mc) => {
                Some(ClipInfo {
                    id: mc.id.clone(),
                    name: mc.clip.name.clone(),
                    kind: "midi".into(),
                    position_ticks: clip.position_ticks,
                    length_ticks: clip.length_ticks,
                    muted: false,
                })
            }
        }
    }).collect()
}

#[derive(Serialize)]
pub struct ClipInfo {
    id: String,
    name: String,
    kind: String,
    position_ticks: u64,
    length_ticks: u64,
    muted: bool,
}
