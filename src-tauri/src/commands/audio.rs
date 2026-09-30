use crate::AppState;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::State;

/// SHA-256 of a file, streamed so a long sample does not land in memory
/// twice. Returns an empty string when the file cannot be read: the hash is
/// there to help find a file that moved later, and failing to compute it must
/// never block the import itself.
fn hash_source_file(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut hasher = Sha256::new();
    if std::io::copy(&mut file, &mut hasher).is_err() {
        return String::new();
    }
    format!("{:x}", hasher.finalize())
}

#[derive(Serialize)]
pub struct ImportedClip {
    track_id: String,
    /// Readable by other commands: freezing has to find the clip it just
    /// imported to move it onto the freeze lane.
    pub(crate) clip_id: String,
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
        source_hash: hash_source_file(&path),
        source_start: 0,
        source_end: info.total_frames,
        gain_db: 0.0,
        fade_in_ticks: 0,
        fade_out_ticks: 0,
        muted: false,
        reversed: false,
        pitch_semitones: 0.0,
        stretch_ratio: 1.0,
        warp_markers: Vec::new(),
        fade_in_curve: Default::default(),
        fade_out_curve: Default::default(),
        // Where the audio really is. `source_path` above is the pool id, so
        // without this the project keeps no record of the file and every clip
        // reloads silent.
        source_file: path.to_string_lossy().into_owned(),
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

/// Map a project-side `Track` into the serialized `ClipInfo` vector
/// the frontend expects. Shared by `get_track_clips` and the bulk
/// `get_tracks_with_clips` endpoint.
pub(crate) fn track_clips_to_info(track: &hardwave_project::Track) -> Vec<ClipInfo> {
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
                warp_markers: ac.warp_markers.clone(),
                fade_in_curve: fade_curve_name(ac.fade_in_curve),
                fade_out_curve: fade_curve_name(ac.fade_out_curve),
                lane: clip.lane,
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
                warp_markers: Vec::new(),
                fade_in_curve: "linear".into(),
                fade_out_curve: "linear".into(),
                lane: clip.lane,
            },
        })
        .collect()
}

/// Get clips for a specific track.
#[tauri::command]
pub fn get_track_clips(state: State<AppState>, track_id: String) -> Vec<ClipInfo> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    match project.track(&track_id) {
        Some(t) => track_clips_to_info(t),
        None => vec![],
    }
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
    /// Piecewise timeline→source warp anchors (empty = plain stretch).
    #[serde(rename = "warpMarkers")]
    warp_markers: Vec<hardwave_project::clip::WarpMarker>,
    #[serde(rename = "fadeInCurve")]
    fade_in_curve: String,
    #[serde(rename = "fadeOutCurve")]
    fade_out_curve: String,
    /// Which take lane this piece sits on. 0 unless the takes on the
    /// track have been spread out for comping.
    lane: u32,
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

/// Clamp pitch to the roadmap-documented range of ±24 semitones.
pub fn clamp_clip_pitch_semitones(pitch_semitones: f64) -> f64 {
    pitch_semitones.clamp(-24.0, 24.0)
}

/// Clamp stretch ratio to a strict superset of the roadmap-documented
/// 50%..200% range. 1.0 = realtime, 0.25 = 4× slower, 4.0 = 4× faster.
pub fn clamp_clip_stretch_ratio(stretch_ratio: f64) -> f64 {
    stretch_ratio.clamp(0.25, 4.0)
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
        ac.pitch_semitones = clamp_clip_pitch_semitones(pitch_semitones);
    })?;
    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Detect transient positions in a source buffer — the "Detect" action
/// in the warp editor. Returns onset positions as SOURCE samples,
/// ascending; the UI turns them into warp-marker suggestions (mapping
/// each onset to its current timeline tick so warping starts neutral).
#[tauri::command]
pub fn detect_clip_transients(
    state: State<AppState>,
    source_id: String,
) -> Result<Vec<u64>, String> {
    let engine = state.engine.lock();
    let buffer = engine
        .audio_pool
        .get(&source_id)
        .ok_or_else(|| format!("Source not found: {}", source_id))?;
    // Mono mix for detection; onset positions are channel-agnostic.
    let mono: Vec<f32> = (0..buffer.num_frames)
        .map(|i| {
            let l = buffer.sample(0, i);
            let r = if buffer.channels.len() > 1 {
                buffer.sample(1, i)
            } else {
                l
            };
            (l + r) * 0.5
        })
        .collect();
    Ok(hardwave_dsp::onset::detect_onsets(
        &mono,
        buffer.sample_rate,
    ))
}

/// Replace a clip's full warp-marker set in one call. The UI edits the
/// whole list locally (drag/add/remove) and commits atomically — a
/// single command avoids marker-index races between rapid drags, and
/// undo snapshots one coherent mutation. Markers are normalized here
/// (sorted by clip_tick, per-tick duplicates dropped) so the engine's
/// segment builder always sees a canonical list. An empty list removes
/// all warping (clip falls back to plain stretch_ratio).
#[tauri::command]
pub fn set_clip_warp_markers(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    markers: Vec<hardwave_project::clip::WarpMarker>,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    with_audio_clip_mut(&engine, &track_id, &clip_id, |ac| {
        let mut m = markers;
        m.sort_by_key(|w| w.clip_tick);
        m.dedup_by_key(|w| w.clip_tick);
        ac.warp_markers = m;
    })?;
    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Pin every detected beat in an audio clip to the nearest grid line.
///
/// Warp markers, the piecewise map from timeline position to position in the
/// file, have been in the engine and the project format since they were
/// written, with a command to set them and nothing that ever called it: a
/// loop at the wrong tempo could only be stretched as a whole.
///
/// This is the automatic half of warping. It finds the transients, works out
/// where each one currently falls in the song, snaps that to the grid, and
/// writes the result as markers. A marker is only kept when the beat is not
/// already on the grid, so a loop that is already in time is left alone
/// rather than pinned to itself a hundred times.
///
/// `grid_ticks` is the playlist's snap setting, in ticks.
/// Returns how many markers were written.
#[tauri::command]
pub fn warp_clip_to_grid(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    grid_ticks: u64,
) -> Result<usize, String> {
    if grid_ticks == 0 {
        return Err("warping needs a grid; turn snap on first".into());
    }
    let engine = state.engine.lock();
    let sample_rate = engine.current_sample_rate() as f64;

    // What the clip plays, and where it sits.
    let (source_id, source_start, source_end, position_ticks, length_ticks, stretch) = {
        let project = engine.project.lock();
        let track = project
            .track(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let placement = track
            .clips
            .iter()
            .find(|c| match &c.content {
                hardwave_project::clip::ClipContent::Audio(ac) => ac.id == clip_id,
                _ => false,
            })
            .ok_or_else(|| format!("Audio clip not found: {clip_id}"))?;
        let hardwave_project::clip::ClipContent::Audio(ac) = &placement.content else {
            return Err("that clip is not audio".into());
        };
        (
            ac.source_path.clone(),
            ac.source_start,
            ac.source_end,
            placement.position_ticks,
            placement.length_ticks,
            ac.stretch_ratio.max(0.01),
        )
    };

    let buffer = engine
        .audio_pool
        .get(&source_id)
        .ok_or_else(|| "that clip's audio is not loaded".to_string())?;
    let mono: Vec<f32> = if buffer.channels.len() > 1 {
        buffer.channels[0]
            .iter()
            .zip(buffer.channels[1].iter())
            .map(|(l, r)| (l + r) * 0.5)
            .collect()
    } else {
        buffer.channels.first().cloned().unwrap_or_default()
    };
    let onsets = hardwave_dsp::onset::detect_onsets(&mono, buffer.sample_rate);

    // Each onset's current place in the song, then snapped to the grid.
    let markers = {
        let project = engine.project.lock();
        let map = &project.tempo_map;
        let clip_start_samples = map.tick_to_samples(position_ticks, sample_rate);
        let mut out: Vec<hardwave_project::clip::WarpMarker> = Vec::new();
        for onset in onsets {
            if onset < source_start || (source_end > 0 && onset >= source_end) {
                continue;
            }
            // Where it sounds now: its distance into the file, slowed or
            // sped by the clip's stretch, measured from the clip's start.
            let into_clip = (onset - source_start) as f64 / stretch;
            let absolute = clip_start_samples + into_clip.round() as u64;
            let tick = map.samples_to_tick(absolute, sample_rate);
            let clip_tick = tick.saturating_sub(position_ticks);
            if clip_tick >= length_ticks {
                continue;
            }
            let snapped = ((clip_tick as f64 / grid_ticks as f64).round() as u64) * grid_ticks;
            if snapped >= length_ticks {
                continue;
            }
            // Already on the grid: nothing to pull.
            if snapped.abs_diff(clip_tick) == 0 {
                continue;
            }
            out.push(hardwave_project::clip::WarpMarker {
                clip_tick: snapped,
                source_sample: onset,
            });
        }
        out.sort_by_key(|m| m.clip_tick);
        out.dedup_by_key(|m| m.clip_tick);
        out
    };

    if markers.is_empty() {
        return Err(
            "no beats to move: either none were found, or they already sit on the grid".into(),
        );
    }

    drop(engine);
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let count = markers.len();
    with_audio_clip_mut(&engine, &track_id, &clip_id, |ac| {
        ac.warp_markers = markers;
    })?;
    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(count)
}

/// Set clip time-stretch ratio (range 0.25..4.0). 1.0 = realtime./// Set clip time-stretch ratio (range 0.25..4.0). 1.0 = realtime.
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
        ac.stretch_ratio = clamp_clip_stretch_ratio(stretch_ratio);
    })?;
    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_clamp_covers_two_octaves_each_way() {
        assert_eq!(clamp_clip_pitch_semitones(0.0), 0.0);
        assert_eq!(clamp_clip_pitch_semitones(24.0), 24.0);
        assert_eq!(clamp_clip_pitch_semitones(-24.0), -24.0);
        // Out-of-range values clamp to the ±24 st limit.
        assert_eq!(clamp_clip_pitch_semitones(50.0), 24.0);
        assert_eq!(clamp_clip_pitch_semitones(-120.0), -24.0);
        // Non-integer / fine-grained values pass through untouched until the
        // boundary — the dedicated fine-cents pathway lives separately.
        assert_eq!(clamp_clip_pitch_semitones(7.5), 7.5);
        assert_eq!(clamp_clip_pitch_semitones(-3.25), -3.25);
    }

    #[test]
    fn stretch_clamp_is_superset_of_roadmap_range() {
        // Roadmap claims 50%..200% must work. We ship 25%..400%, so both
        // boundary values of the documented range pass through untouched.
        assert_eq!(clamp_clip_stretch_ratio(0.5), 0.5);
        assert_eq!(clamp_clip_stretch_ratio(2.0), 2.0);
        assert_eq!(clamp_clip_stretch_ratio(1.0), 1.0);
        // Our own limits (0.25 / 4.0) clamp values beyond them.
        assert_eq!(clamp_clip_stretch_ratio(0.25), 0.25);
        assert_eq!(clamp_clip_stretch_ratio(4.0), 4.0);
        assert_eq!(clamp_clip_stretch_ratio(0.01), 0.25);
        assert_eq!(clamp_clip_stretch_ratio(10.0), 4.0);
    }
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
/// A small waveform for a file in the browser, without importing it.
///
/// The browser listed sample packs as names. Picking a kick out of two
/// hundred meant auditioning them one at a time, when the shape of the
/// sound says most of it at a glance.
///
/// The file is decoded once and kept in the engine's pool, which is where
/// previewing it would put it anyway, so browsing a folder and then playing
/// one of its files does not decode twice.
#[tauri::command]
pub fn get_file_peaks(
    state: State<AppState>,
    file_path: String,
    num_buckets: usize,
) -> Result<Vec<[f32; 2]>, String> {
    let path = PathBuf::from(&file_path);
    let engine = state.engine.lock();
    let source_id = hardwave_engine::engine::source_id_for_path(&path.to_string_lossy());
    if !engine.audio_pool.contains(&source_id) {
        engine.load_audio_file_as(&path, &source_id)?;
    }
    let buffer = engine
        .audio_pool
        .get(&source_id)
        .ok_or_else(|| format!("could not read {file_path}"))?;
    let frames = buffer.num_frames;
    if frames == 0 || num_buckets == 0 {
        return Ok(Vec::new());
    }
    let bucket = (frames as f64 / num_buckets as f64).ceil().max(1.0) as usize;
    let left = buffer.channels.first();
    let right = buffer.channels.get(1);
    let mut out = Vec::with_capacity(num_buckets);
    let mut start = 0usize;
    while start < frames {
        let end = (start + bucket).min(frames);
        let mut low = 0.0f32;
        let mut high = 0.0f32;
        for i in start..end {
            let l = left.and_then(|c| c.get(i)).copied().unwrap_or(0.0);
            let r = right.and_then(|c| c.get(i)).copied().unwrap_or(l);
            let sample = (l + r) * 0.5;
            if sample < low {
                low = sample;
            }
            if sample > high {
                high = sample;
            }
        }
        out.push([low, high]);
        start = end;
    }
    Ok(out)
}

#[tauri::command]
pub fn get_waveform_peaks(
    state: State<AppState>,
    source_id: String,
    num_buckets: usize,
) -> Result<Vec<[f32; 4]>, String> {
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

    // Each bucket is [min, max, rms, brightness]:
    //   * min/max form the outer peak envelope (the transient "hair"),
    //   * rms forms the brighter inner body — the two-layer waveform DAWs
    //     like FL Studio / rekordbox draw,
    //   * brightness (0..1) is a cheap spectral-tilt estimate driving the
    //     frequency colour of the slice: 0 = low/bass (drawn red), 1 =
    //     high/treble (drawn blue). We estimate it from the RMS of the
    //     first difference (x[n]-x[n-1], a one-tap high-pass) relative to
    //     the signal RMS — no FFT needed, and it tracks dominant frequency
    //     well enough for a per-slice colour. Kick bodies read red, cymbal
    //     / hat transients read blue.
    let num_ch = buffer.channels.len();
    let inv_ch = 1.0 / num_ch as f32;
    for i in 0..num_buckets {
        let start = i * bucket_size;
        let end = ((i + 1) * bucket_size).min(num_frames);
        if start >= num_frames {
            peaks.push([0.0, 0.0, 0.0, 0.0]);
            continue;
        }

        let mut min_val: f32 = 0.0;
        let mut max_val: f32 = 0.0;
        let mut sum_sq: f64 = 0.0;
        let mut diff_sq: f64 = 0.0;

        let mixed = |frame: usize| -> f32 {
            let mut s = 0.0_f32;
            for ch in 0..num_ch {
                s += buffer.sample(ch, frame);
            }
            s * inv_ch
        };

        // Seed prev with the frame before the bucket (or the first sample)
        // so the difference is continuous across bucket boundaries.
        let mut prev = mixed(start.saturating_sub(1));
        for frame in start..end {
            let sample = mixed(frame);
            min_val = min_val.min(sample);
            max_val = max_val.max(sample);
            sum_sq += (sample as f64) * (sample as f64);
            let d = (sample - prev) as f64;
            diff_sq += d * d;
            prev = sample;
        }

        let count = (end - start).max(1) as f64;
        let rms = (sum_sq / count).sqrt();
        let diff_rms = (diff_sq / count).sqrt();
        // diff_rms/rms → 0 for a DC/low tone, ~2 near Nyquist. Fold into a
        // 0..1 brightness: diff/(rms+diff) sits in 0..~0.66, so scale by
        // 1.5 and clamp. Silence stays at 0 (waveform height is ~0 anyway).
        let brightness = if rms + diff_rms > 1e-9 {
            ((diff_rms / (rms + diff_rms)) * 1.5).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        peaks.push([min_val, max_val, rms as f32, brightness]);
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

/// Move a clip from one track to another at a new position.
///
/// Both tracks must have a kind compatible with the clip content
/// (audio clip ↔ audio track, midi clip ↔ midi track). Refuses the
/// move otherwise so a stray vertical drag onto an audio track does
/// not corrupt a midi-bearing chain.
#[tauri::command]
pub fn move_clip_to_track(
    state: State<AppState>,
    from_track_id: String,
    to_track_id: String,
    clip_id: String,
    new_position_ticks: u64,
) -> Result<(), String> {
    if from_track_id == to_track_id {
        // Degenerate case — fall through to the position-only mover so
        // the audio thread does the cheap path.
        return move_clip(state, from_track_id, clip_id, new_position_ticks);
    }
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();

    // Pluck the clip out of the source track first so we hold no
    // borrow when we touch the destination. Audio + Midi content
    // types use independent id strings; check both shapes.
    let src = project
        .track_mut(&from_track_id)
        .ok_or_else(|| format!("Source track not found: {}", from_track_id))?;
    let idx = src
        .clips
        .iter()
        .position(|c| match &c.content {
            hardwave_project::clip::ClipContent::Audio(ac) => ac.id == clip_id,
            hardwave_project::clip::ClipContent::Midi(mc) => mc.id == clip_id,
        })
        .ok_or_else(|| format!("Clip not found on source track: {}", clip_id))?;
    let mut clip = src.clips.remove(idx);
    clip.position_ticks = new_position_ticks;

    let clip_is_audio = matches!(&clip.content, hardwave_project::clip::ClipContent::Audio(_));

    let dst = project
        .track_mut(&to_track_id)
        .ok_or_else(|| format!("Destination track not found: {}", to_track_id))?;
    let dst_is_audio = matches!(dst.kind, hardwave_project::track::TrackKind::Audio);
    let dst_is_midi = matches!(dst.kind, hardwave_project::track::TrackKind::Midi);
    if clip_is_audio && !dst_is_audio {
        return Err("Cannot move audio clip to a non-audio track".into());
    }
    if !clip_is_audio && !dst_is_midi {
        return Err("Cannot move MIDI clip to a non-MIDI track".into());
    }
    dst.clips.push(clip);

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

/// Mute or unmute one clip.
///
/// The playlist's mute tool was keybound and documented but fell through to
/// the default, so clicking a clip with it selected did nothing at all: there
/// was no command behind it. The engine already skips muted clips, so this is
/// the whole feature.
#[tauri::command]
pub fn set_clip_muted(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    muted: bool,
) -> Result<bool, String> {
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    {
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let clip = track
            .clips
            .iter_mut()
            .find(|c| match &c.content {
                hardwave_project::clip::ClipContent::Audio(ac) => ac.id == clip_id,
                hardwave_project::clip::ClipContent::Midi(mc) => mc.id == clip_id,
            })
            .ok_or_else(|| format!("Clip not found: {clip_id}"))?;
        match &mut clip.content {
            hardwave_project::clip::ClipContent::Audio(ac) => ac.muted = muted,
            hardwave_project::clip::ClipContent::Midi(mc) => mc.clip.muted = muted,
        }
    }
    engine.rebuild_graph();
    Ok(muted)
}

/// Slide the audio inside a clip while the clip itself stays put.
///
/// The slip tool was in the same state as mute: keybound, documented, and
/// with nothing behind it. Slipping moves the window into the source, so the
/// clip keeps its position and length on the timeline and plays a different
/// part of the sample. A negative delta slips earlier.
///
/// Clamped at the start of the source, because slipping before sample zero
/// would silently play nothing and look like a broken clip. Returns the new
/// source offset so the UI can show where it landed.
#[tauri::command]
pub fn slip_clip(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    delta_samples: i64,
) -> Result<u64, String> {
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    let new_start;
    {
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let clip = track
            .clips
            .iter_mut()
            .find(|c| match &c.content {
                hardwave_project::clip::ClipContent::Audio(ac) => ac.id == clip_id,
                hardwave_project::clip::ClipContent::Midi(mc) => mc.id == clip_id,
            })
            .ok_or_else(|| format!("Clip not found: {clip_id}"))?;
        match &mut clip.content {
            hardwave_project::clip::ClipContent::Audio(ac) => {
                let window = ac.source_end.saturating_sub(ac.source_start);
                let start = if delta_samples >= 0 {
                    ac.source_start.saturating_add(delta_samples as u64)
                } else {
                    ac.source_start.saturating_sub(delta_samples.unsigned_abs())
                };
                ac.source_start = start;
                // Keep the window the same length: slipping changes which
                // part of the sample plays, not how long the clip is.
                ac.source_end = start.saturating_add(window);
                new_start = start;
            }
            hardwave_project::clip::ClipContent::Midi(_) => {
                return Err("Slip applies to audio clips".to_string());
            }
        }
    }
    engine.rebuild_graph();
    Ok(new_start)
}

/// Audition a file through the engine.
///
/// The browser used to preview with an HTML `Audio` element inside the
/// WebView, which plays through whatever output the browser considers
/// default. That is not the device the DAW holds: on ASIO or WASAPI-exclusive
/// the audition was silent or came out of the wrong speakers, and it ignored
/// the preview volume the DAW shows. Loading into the audio pool also means
/// the file is decoded by the same decoder that will play it on a track.
#[tauri::command]
pub fn preview_audio_file(state: State<AppState>, file_path: String) -> Result<(), String> {
    state.engine.lock().preview_file(&PathBuf::from(&file_path))
}

/// What the DAW can work out about a file before importing it: the tempo
/// written in its name, and what that means for auditioning it against the
/// song.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPlan {
    /// Tempo read from the file name, or null when the name does not say.
    pub file_bpm: Option<f64>,
    /// Speed the preview will run at: 1.0 is the file's own speed.
    pub speed: f64,
}

/// Read a tempo out of a file name: "Kick 150.wav", "loop_128bpm.wav",
/// "174 dnb break.wav".
///
/// Names are all a loop usually carries. Reading it is a guess, which is
/// why the UI says where the number came from rather than silently
/// repitching someone's sample.
fn bpm_from_name(name: &str) -> Option<f64> {
    let lower = name.to_lowercase();
    let bytes = lower.as_bytes();
    let mut best: Option<f64> = None;
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let digits = &lower[start..i];
        // A sample rate or a date is not a tempo.
        if digits.len() > 3 {
            continue;
        }
        let Ok(value) = digits.parse::<f64>() else {
            continue;
        };
        if !(60.0..=220.0).contains(&value) {
            continue;
        }
        // "128bpm" is a tempo for certain; a bare number is a guess, so a
        // labelled one always wins.
        let labelled = lower[i..].trim_start().starts_with("bpm");
        if labelled {
            return Some(value);
        }
        if best.is_none() {
            best = Some(value);
        }
    }
    best
}

/// Preview a file, matched to the song's tempo when the file name says what
/// tempo it is.
///
/// Loops previewed at their own speed are hard to judge: the only way to
/// tell whether a 128 BPM loop fits a 150 BPM song was to import it. This
/// speeds the preview up or down to the song's tempo, which also moves the
/// pitch, the way a turntable does. It is not time stretching and the UI
/// says so.
#[tauri::command]
pub fn preview_audio_file_in_tempo(
    state: State<AppState>,
    file_path: String,
    match_tempo: bool,
) -> Result<PreviewPlan, String> {
    let path = PathBuf::from(&file_path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let file_bpm = bpm_from_name(&name);
    let engine = state.engine.lock();
    let song_bpm = {
        let project = engine.project.lock();
        project
            .tempo_map
            .entries
            .first()
            .map(|e| e.bpm)
            .unwrap_or(120.0)
    };
    let speed = match (match_tempo, file_bpm) {
        (true, Some(bpm)) if bpm > 0.0 => (song_bpm / bpm).clamp(0.25, 4.0),
        _ => 1.0,
    };
    engine.preview_file_at_speed(&path, speed)?;
    Ok(PreviewPlan { file_bpm, speed })
}

#[tauri::command]
pub fn stop_audio_preview(state: State<AppState>) {
    state.engine.lock().preview().stop_playing();
}

#[tauri::command]
pub fn set_preview_volume(state: State<AppState>, volume: f32) {
    state.engine.lock().preview().set_volume(volume);
}

#[cfg(test)]
mod preview_tempo_tests {
    use super::bpm_from_name;

    #[test]
    fn a_labelled_tempo_wins_over_any_other_number() {
        assert_eq!(bpm_from_name("track 3 loop_128bpm.wav"), Some(128.0));
        assert_eq!(bpm_from_name("02 - 174 BPM dnb break.wav"), Some(174.0));
    }

    #[test]
    fn a_bare_number_in_range_is_taken_as_the_tempo() {
        assert_eq!(bpm_from_name("Kick 150.wav"), Some(150.0));
        assert_eq!(bpm_from_name("hardstyle 155 lead.wav"), Some(155.0));
    }

    #[test]
    fn numbers_that_are_not_tempos_are_left_alone() {
        // A sample rate, a year, a take number, a note name.
        assert_eq!(bpm_from_name("vocal 44100.wav"), None);
        assert_eq!(bpm_from_name("session 2026 master.wav"), None);
        assert_eq!(bpm_from_name("Take 3.wav"), None);
        assert_eq!(bpm_from_name("Screech F.wav"), None);
    }
}
