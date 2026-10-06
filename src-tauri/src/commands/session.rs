//! The clip launcher.
//!
//! A timeline is for writing a song down. A grid of loops is for
//! finding one, and for playing live: press a clip and it starts on
//! the next bar, press a row and the row starts together.

use crate::AppState;
use serde::Serialize;
use tauri::State;

/// One cell of the grid, as the UI needs it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotInfo {
    pub name: String,
    pub length_ticks: u64,
}

/// A track's row, and what it is doing.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackRow {
    pub track_id: String,
    pub name: String,
    pub slots: Vec<Option<SlotInfo>>,
    /// The slot sounding now, if any.
    pub playing: Option<usize>,
    /// What has been asked for and is waiting for the bar: a slot
    /// index, or -2 for a stop that is waiting.
    pub pending: i32,
}

/// The whole grid.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionGrid {
    pub scenes: Vec<String>,
    pub rows: Vec<TrackRow>,
    /// How a launch is lined up, in beats.
    pub quantise_beats: u32,
}

#[tauri::command]
pub fn get_session_grid(state: State<AppState>) -> SessionGrid {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    let project = engine.project.lock();
    let scene_count = project.scenes.len();
    let rows = project
        .tracks
        .iter()
        .filter(|t| !matches!(t.kind, hardwave_project::track::TrackKind::Master))
        .map(|track| {
            let session = engine.session.track(&track.id);
            let slots = (0..scene_count)
                .map(|i| {
                    track
                        .session_slots
                        .get(i)
                        .and_then(|s| s.as_ref())
                        .map(|s| SlotInfo {
                            name: s.name.clone(),
                            length_ticks: s.length_ticks,
                        })
                })
                .collect();
            TrackRow {
                track_id: track.id.clone(),
                name: track.name.clone(),
                slots,
                playing: session.playing_slot(),
                pending: session.pending_slot(),
            }
        })
        .collect();
    SessionGrid {
        scenes: project.scenes.clone(),
        rows,
        quantise_beats: engine.session.quantise_beats.load(Ordering::Relaxed),
    }
}

/// Press a clip. It starts at the next boundary, which is what keeps
/// it in time with whatever is already going.
#[tauri::command]
pub fn launch_slot(state: State<AppState>, track_id: String, slot: usize) -> Result<(), String> {
    let engine = state.engine.lock();
    {
        let project = engine.project.lock();
        let track = project
            .track(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        if track
            .session_slots
            .get(slot)
            .and_then(|s| s.as_ref())
            .is_none()
        {
            return Err("there is no clip in that slot".into());
        }
    }
    engine.session.track(&track_id).queue(slot as i32);
    Ok(())
}

/// Stop a track at the next boundary, so it ends with the bar rather
/// than in the middle of one.
#[tauri::command]
pub fn stop_slot(state: State<AppState>, track_id: String) {
    state.engine.lock().session.track(&track_id).queue_stop();
}

/// Launch a whole row. Tracks with nothing in it stop, which is what
/// makes a row a section rather than a pile.
#[tauri::command]
pub fn launch_scene(state: State<AppState>, scene: usize) {
    let engine = state.engine.lock();
    let has_clip: std::collections::HashSet<String> = {
        let project = engine.project.lock();
        project
            .tracks
            .iter()
            .filter(|t| {
                t.session_slots
                    .get(scene)
                    .and_then(|s| s.as_ref())
                    .is_some()
            })
            .map(|t| t.id.clone())
            .collect()
    };
    // Every track with a row needs a state before the scene can reach
    // it, or a track that has never been touched would be missed.
    {
        let project = engine.project.lock();
        for track in &project.tracks {
            let _ = engine.session.track(&track.id);
        }
    }
    engine
        .session
        .queue_scene(scene, |id| has_clip.contains(id));
}

/// Stop everything in the grid, now rather than at the bar: a stop
/// button that waits is not a stop button.
#[tauri::command]
pub fn stop_all_slots(state: State<AppState>) {
    state.engine.lock().session.stop_all();
}

/// How a launch is lined up, in beats. Four is a bar in common time.
#[tauri::command]
pub fn set_session_quantise(state: State<AppState>, beats: u32) {
    state
        .engine
        .lock()
        .session
        .quantise_beats
        .store(beats.min(16), std::sync::atomic::Ordering::Relaxed);
}

/// Put a clip into a slot, from an audio file.
#[tauri::command]
pub fn set_session_slot(
    state: State<AppState>,
    track_id: String,
    slot: usize,
    path: String,
    name: Option<String>,
) -> Result<u64, String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let sample_rate = engine.current_sample_rate() as f64;

    // The file has to be in the pool before a slot can point at it.
    let frames = match engine.audio_pool.get(&path) {
        Some(buffer) => buffer.num_frames,
        None => {
            let (info, channels) = hardwave_dsp::audio_file::AudioFileReader::read_resampled(
                std::path::Path::new(&path),
                Some(sample_rate as u32),
            )
            .map_err(|e| format!("could not read that file: {e}"))?;
            let frames = channels.first().map(|c| c.len()).unwrap_or(0);
            engine.audio_pool.insert(
                path.clone(),
                hardwave_engine::audio_pool::AudioBuffer {
                    channels,
                    sample_rate: info.sample_rate,
                    num_frames: frames,
                },
            );
            frames
        }
    };

    let bpm = engine
        .transport
        .bpm
        .load(std::sync::atomic::Ordering::Relaxed);
    let seconds = frames as f64 / sample_rate;
    let length_ticks = (seconds * bpm / 60.0 * hardwave_midi::PPQ as f64).round() as u64;
    let length_ticks = length_ticks.max(1);

    {
        let mut project = engine.project.lock();
        let scene_count = project.scenes.len();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        if track.session_slots.len() < scene_count.max(slot + 1) {
            track.session_slots.resize(scene_count.max(slot + 1), None);
        }
        track.session_slots[slot] = Some(hardwave_project::track::SessionSlot {
            name: name.unwrap_or_else(|| {
                std::path::Path::new(&path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Clip")
                    .to_string()
            }),
            source_path: path,
            source_start: 0,
            length_ticks,
            gain_db: 0.0,
        });
    }
    engine.rebuild_graph();
    Ok(length_ticks)
}

/// Empty a slot.
#[tauri::command]
pub fn clear_session_slot(state: State<AppState>, track_id: String, slot: usize) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            if let Some(cell) = track.session_slots.get_mut(slot) {
                *cell = None;
            }
        }
    }
    // A slot that is playing and is then emptied should stop, rather
    // than keep looping audio the grid no longer shows.
    let session = engine.session.track(&track_id);
    if session.playing_slot() == Some(slot) {
        session.stop_now();
    }
    engine.rebuild_graph();
}

/// Add a row to the grid.
#[tauri::command]
pub fn add_scene(state: State<AppState>, name: Option<String>) -> usize {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let index = project.scenes.len();
    project
        .scenes
        .push(name.unwrap_or_else(|| format!("Scene {}", index + 1)));
    index
}
