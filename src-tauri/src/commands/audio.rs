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
    state.engine.lock().snapshot_before_mutation();
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
        reversed: false,
        pitch_semitones: 0.0,
        stretch_ratio: 1.0,
        fade_in_curve: Default::default(),
        fade_out_curve: Default::default(),
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
                gain_db: ac.gain_db,
                fade_in_ticks: ac.fade_in_ticks,
                fade_out_ticks: ac.fade_out_ticks,
                reversed: ac.reversed,
                pitch_semitones: ac.pitch_semitones,
                stretch_ratio: ac.stretch_ratio,
                fade_in_curve: fade_curve_name(ac.fade_in_curve),
                fade_out_curve: fade_curve_name(ac.fade_out_curve),
            },
            hardwave_project::clip::ClipContent::Midi(mc) => ClipInfo {
                id: mc.id.clone(),
                name: mc.clip.name.clone(),
                kind: "midi".into(),
                source_id: String::new(),
                position_ticks: clip.position_ticks,
                length_ticks: clip.length_ticks,
                muted: false,
                gain_db: 0.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                fade_in_curve: "linear".into(),
                fade_out_curve: "linear".into(),
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
    #[serde(rename = "gainDb")]
    gain_db: f64,
    #[serde(rename = "fadeInTicks")]
    fade_in_ticks: u64,
    #[serde(rename = "fadeOutTicks")]
    fade_out_ticks: u64,
    reversed: bool,
    #[serde(rename = "pitchSemitones")]
    pitch_semitones: f64,
    #[serde(rename = "stretchRatio")]
    stretch_ratio: f64,
    #[serde(rename = "fadeInCurve")]
    fade_in_curve: String,
    #[serde(rename = "fadeOutCurve")]
    fade_out_curve: String,
}

fn fade_curve_name(curve: hardwave_project::clip::FadeCurve) -> String {
    use hardwave_project::clip::FadeCurve;
    match curve {
        FadeCurve::Linear => "linear",
        FadeCurve::EqualPower => "equal_power",
        FadeCurve::SCurve => "s_curve",
        FadeCurve::Logarithmic => "logarithmic",
    }
    .into()
}

fn fade_curve_from_name(name: &str) -> Result<hardwave_project::clip::FadeCurve, String> {
    use hardwave_project::clip::FadeCurve;
    Ok(match name {
        "linear" => FadeCurve::Linear,
        "equal_power" => FadeCurve::EqualPower,
        "s_curve" => FadeCurve::SCurve,
        "logarithmic" => FadeCurve::Logarithmic,
        other => return Err(format!("Unknown fade curve: {other}")),
    })
}

fn with_audio_clip_mut<R>(
    engine: &hardwave_engine::DawEngine,
    track_id: &str,
    clip_id: &str,
    f: impl FnOnce(&mut hardwave_project::clip::AudioClip) -> R,
) -> Result<R, String> {
    let mut project = engine.project.lock();
    let track = project
        .track_mut(track_id)
        .ok_or_else(|| format!("Track not found: {track_id}"))?;
    let clip = track
        .clips
        .iter_mut()
        .find_map(|c| match &mut c.content {
            hardwave_project::clip::ClipContent::Audio(ac) if ac.id == clip_id => Some(ac),
            _ => None,
        })
        .ok_or_else(|| format!("Audio clip not found: {clip_id}"))?;
    Ok(f(clip))
}

/// Set the gain (in dB) of an audio clip.
#[tauri::command]
pub fn set_clip_gain(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    gain_db: f64,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    with_audio_clip_mut(&engine, &track_id, &clip_id, |ac| {
        ac.gain_db = gain_db.clamp(-60.0, 12.0);
    })?;
    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Set the fade-in / fade-out lengths (in ticks) for an audio clip.
#[tauri::command]
pub fn set_clip_fades(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    fade_in_ticks: u64,
    fade_out_ticks: u64,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    // Fades may not sum to more than the clip length — clamp for safety.
    let length_ticks = {
        let project = engine.project.lock();
        project
            .track(&track_id)
            .and_then(|t| {
                t.clips.iter().find_map(|c| match &c.content {
                    hardwave_project::clip::ClipContent::Audio(ac) if ac.id == clip_id => {
                        Some(c.length_ticks)
                    }
                    _ => None,
                })
            })
            .unwrap_or(u64::MAX)
    };
    let total = fade_in_ticks.saturating_add(fade_out_ticks);
    let (fi, fo) = if total > length_ticks {
        // Scale both down proportionally so they just fit.
        let ratio = length_ticks as f64 / total.max(1) as f64;
        (
            (fade_in_ticks as f64 * ratio) as u64,
            (fade_out_ticks as f64 * ratio) as u64,
        )
    } else {
        (fade_in_ticks, fade_out_ticks)
    };
    with_audio_clip_mut(&engine, &track_id, &clip_id, |ac| {
        ac.fade_in_ticks = fi;
        ac.fade_out_ticks = fo;
    })?;
    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Set fade-in and fade-out curve shapes for an audio clip.
#[tauri::command]
pub fn set_clip_fade_curves(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    fade_in_curve: String,
    fade_out_curve: String,
) -> Result<(), String> {
    let fi = fade_curve_from_name(&fade_in_curve)?;
    let fo = fade_curve_from_name(&fade_out_curve)?;
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    with_audio_clip_mut(&engine, &track_id, &clip_id, |ac| {
        ac.fade_in_curve = fi;
        ac.fade_out_curve = fo;
    })?;
    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Set clip pitch shift in semitones (range -24..+24).
#[tauri::command]
pub fn set_clip_pitch(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    pitch_semitones: f64,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    with_audio_clip_mut(&engine, &track_id, &clip_id, |ac| {
        ac.pitch_semitones = pitch_semitones.clamp(-24.0, 24.0);
    })?;
    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Set clip time-stretch ratio (range 0.25..4.0). 1.0 = realtime.
#[tauri::command]
pub fn set_clip_stretch(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    stretch_ratio: f64,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    with_audio_clip_mut(&engine, &track_id, &clip_id, |ac| {
        ac.stretch_ratio = stretch_ratio.clamp(0.25, 4.0);
    })?;
    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Toggle the reverse flag of an audio clip.
#[tauri::command]
pub fn toggle_clip_reverse(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
) -> Result<bool, String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let new_value = with_audio_clip_mut(&engine, &track_id, &clip_id, |ac| {
        ac.reversed = !ac.reversed;
        ac.reversed
    })?;
    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(new_value)
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
    state.engine.lock().snapshot_before_mutation();
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
    state.engine.lock().snapshot_before_mutation();
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

/// Duplicate a clip within the same track, placing the copy immediately after it.
#[tauri::command]
pub fn duplicate_clip(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
) -> Result<String, String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    let original = track
        .clips
        .iter()
        .find(|c| match &c.content {
            hardwave_project::clip::ClipContent::Audio(ac) => ac.id == clip_id,
            hardwave_project::clip::ClipContent::Midi(mc) => mc.id == clip_id,
        })
        .ok_or_else(|| format!("Clip not found: {}", clip_id))?
        .clone();

    let new_id = uuid::Uuid::new_v4().to_string();
    let mut copy = original.clone();
    copy.position_ticks = original.position_ticks + original.length_ticks;
    match &mut copy.content {
        hardwave_project::clip::ClipContent::Audio(ac) => ac.id = new_id.clone(),
        hardwave_project::clip::ClipContent::Midi(mc) => mc.id = new_id.clone(),
    }
    track.clips.push(copy);
    drop(project);
    engine.rebuild_graph();
    Ok(new_id)
}

/// Split a clip at the given absolute timeline tick position.
/// Returns the id of the newly created right-hand clip.
#[tauri::command]
pub fn split_clip(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    at_ticks: u64,
) -> Result<String, String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    let idx = track
        .clips
        .iter()
        .position(|c| match &c.content {
            hardwave_project::clip::ClipContent::Audio(ac) => ac.id == clip_id,
            hardwave_project::clip::ClipContent::Midi(mc) => mc.id == clip_id,
        })
        .ok_or_else(|| format!("Clip not found: {}", clip_id))?;

    let original = track.clips[idx].clone();
    let start = original.position_ticks;
    let end = start + original.length_ticks;
    if at_ticks <= start || at_ticks >= end {
        return Err(format!(
            "split position {} outside clip [{}, {})",
            at_ticks, start, end
        ));
    }
    let first_ticks = at_ticks - start;
    let second_ticks = end - at_ticks;

    // Shrink the original to the left half.
    track.clips[idx].length_ticks = first_ticks;

    // Build the right-hand clip.
    let new_id = uuid::Uuid::new_v4().to_string();
    let mut right = original.clone();
    right.position_ticks = at_ticks;
    right.length_ticks = second_ticks;
    match &mut right.content {
        hardwave_project::clip::ClipContent::Audio(ac) => {
            // Proportionally advance the source_start for the right half.
            let total_src = ac.source_end.saturating_sub(ac.source_start) as u128;
            let offset =
                (total_src * first_ticks as u128 / (original.length_ticks.max(1)) as u128) as u64;
            ac.source_start = ac.source_start.saturating_add(offset);
            ac.id = new_id.clone();
        }
        hardwave_project::clip::ClipContent::Midi(mc) => {
            mc.id = new_id.clone();
        }
    }
    track.clips.push(right);

    drop(project);
    engine.rebuild_graph();
    Ok(new_id)
}

/// Delete a clip from a track.
#[tauri::command]
pub fn delete_clip(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
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
