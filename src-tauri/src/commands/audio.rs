use crate::AppState;
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

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
    let file_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();

    // Load audio into pool
    let engine = state.engine.lock();
    let (source_id, info) = engine.load_audio_file(&path)?;

    // Calculate clip length in ticks based on duration and current BPM
    let bpm = engine
        .transport
        .bpm
        .load(std::sync::atomic::Ordering::Relaxed);
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
pub fn get_track_clips(state: State<AppState>, track_id: String) -> Vec<ClipInfo> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    let track = match project.track(&track_id) {
        Some(t) => t,
        None => return vec![],
    };

    track
        .clips
        .iter()
        .map(|clip| match &clip.content {
            hardwave_project::clip::ClipContent::Audio(ac) => ClipInfo {
                id: ac.id.clone(),
                name: ac.name.clone(),
                kind: "audio".into(),
                source_id: ac.source_path.clone(),
                position_ticks: clip.position_ticks,
                length_ticks: clip.length_ticks,
                muted: ac.muted,
            },
            hardwave_project::clip::ClipContent::Midi(mc) => ClipInfo {
                id: mc.id.clone(),
                name: mc.clip.name.clone(),
                kind: "midi".into(),
                source_id: String::new(),
                position_ticks: clip.position_ticks,
                length_ticks: clip.length_ticks,
                muted: false,
            },
        })
        .collect()
}

#[derive(Serialize)]
pub struct ClipInfo {
    id: String,
    name: String,
    kind: String,
    source_id: String,
    position_ticks: u64,
    length_ticks: u64,
    muted: bool,
}

/// Get downsampled waveform peaks for an audio source.
/// Returns pairs of (min, max) per bucket for rendering.
#[tauri::command]
pub fn get_waveform_peaks(
    state: State<AppState>,
    source_id: String,
    num_buckets: usize,
) -> Result<Vec<[f32; 2]>, String> {
    let engine = state.engine.lock();
    let buffer = engine
        .audio_pool
        .get(&source_id)
        .ok_or_else(|| format!("Source not found: {}", source_id))?;

    let num_frames = buffer.num_frames;
    if num_frames == 0 || num_buckets == 0 {
        return Ok(vec![]);
    }

    let bucket_size = (num_frames as f64 / num_buckets as f64).ceil() as usize;
    let mut peaks = Vec::with_capacity(num_buckets);

    for i in 0..num_buckets {
        let start = i * bucket_size;
        let end = ((i + 1) * bucket_size).min(num_frames);
        if start >= num_frames {
            peaks.push([0.0, 0.0]);
            continue;
        }

        let mut min_val: f32 = 0.0;
        let mut max_val: f32 = 0.0;

        // Mix all channels for the peak display
        let num_ch = buffer.channels.len();
        for frame in start..end {
            let mut sample = 0.0_f32;
            for ch in 0..num_ch {
                sample += buffer.sample(ch, frame);
            }
            sample /= num_ch as f32;
            min_val = min_val.min(sample);
            max_val = max_val.max(sample);
        }

        peaks.push([min_val, max_val]);
    }

    Ok(peaks)
}

/// Move a clip to a new position (in ticks).
#[tauri::command]
pub fn move_clip(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    new_position_ticks: u64,
) -> Result<(), String> {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    let clip = track
        .clips
        .iter_mut()
        .find(|c| match &c.content {
            hardwave_project::clip::ClipContent::Audio(ac) => ac.id == clip_id,
            hardwave_project::clip::ClipContent::Midi(mc) => mc.id == clip_id,
        })
        .ok_or_else(|| format!("Clip not found: {}", clip_id))?;

    clip.position_ticks = new_position_ticks;
    drop(project);
    engine.rebuild_graph();
    Ok(())
}

/// Resize a clip (change its length in ticks).
#[tauri::command]
pub fn resize_clip(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    new_length_ticks: u64,
) -> Result<(), String> {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    let clip = track
        .clips
        .iter_mut()
        .find(|c| match &c.content {
            hardwave_project::clip::ClipContent::Audio(ac) => ac.id == clip_id,
            hardwave_project::clip::ClipContent::Midi(mc) => mc.id == clip_id,
        })
        .ok_or_else(|| format!("Clip not found: {}", clip_id))?;

    clip.length_ticks = new_length_ticks;
    drop(project);
    engine.rebuild_graph();
    Ok(())
}

/// Delete a clip from a track.
#[tauri::command]
pub fn delete_clip(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
) -> Result<(), String> {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    let before = track.clips.len();
    track.clips.retain(|c| match &c.content {
        hardwave_project::clip::ClipContent::Audio(ac) => ac.id != clip_id,
        hardwave_project::clip::ClipContent::Midi(mc) => mc.id != clip_id,
    });

    if track.clips.len() == before {
        return Err(format!("Clip not found: {}", clip_id));
    }

    drop(project);
    engine.rebuild_graph();
    Ok(())
}
