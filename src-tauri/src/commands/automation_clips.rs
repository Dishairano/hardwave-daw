//! Automation-clip commands — arrangement-level automation as movable
//! playlist objects (FL Studio's automation clips). The data model lives
//! in `hardwave_project::automation_clip`; these commands place clips on
//! a track, edit their points, and trigger an engine rebuild so the audio
//! thread evaluates them (see `track_node.rs`). Clip points are stored in
//! clip-local tick space (0..length_ticks).

use crate::commands::automation::LaneTargetSpec;
use crate::AppState;
use hardwave_project::automation::CurveMode;
use hardwave_project::automation_clip::AutomationClip;
use tauri::State;
use uuid::Uuid;

/// Run `f` against a track's automation-clip list with the standard
/// snapshot → mutate → rebuild discipline.
fn with_clips<T>(
    state: &State<AppState>,
    track_id: &str,
    f: impl FnOnce(&mut Vec<AutomationClip>) -> Result<T, String>,
) -> Result<T, String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let result = {
        let mut project = engine.project.lock();
        let track = project
            .track_mut(track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        f(&mut track.automation_clips)
    };
    engine.rebuild_graph();
    result
}

/// Create an automation clip on a track targeting a parameter. Returns
/// the new clip id.
#[tauri::command]
pub fn create_automation_clip(
    state: State<AppState>,
    track_id: String,
    target: LaneTargetSpec,
    start_tick: u64,
    length_ticks: u64,
) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let id_out = id.clone();
    with_clips(&state, &track_id, |clips| {
        clips.push(AutomationClip::new(
            id,
            target.into(),
            start_tick,
            length_ticks.max(1),
        ));
        Ok(())
    })?;
    Ok(id_out)
}

#[tauri::command]
pub fn delete_automation_clip(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
) -> Result<(), String> {
    with_clips(&state, &track_id, |clips| {
        clips.retain(|c| c.id != clip_id);
        Ok(())
    })
}

/// Every automation clip on a track, serialized for the playlist to
/// render (the `AutomationClip` struct is `Serialize`).
#[tauri::command]
pub fn list_automation_clips(state: State<AppState>, track_id: String) -> Vec<AutomationClip> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project
        .track(&track_id)
        .map(|t| t.automation_clips.clone())
        .unwrap_or_default()
}

/// Drag a clip along the timeline.
#[tauri::command]
pub fn move_automation_clip(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    start_tick: u64,
) -> Result<(), String> {
    with_clips(&state, &track_id, |clips| {
        let clip = clips
            .iter_mut()
            .find(|c| c.id == clip_id)
            .ok_or_else(|| format!("Automation clip not found: {clip_id}"))?;
        clip.start_tick = start_tick;
        Ok(())
    })
}

/// Resize a clip (its point ticks are clip-local, so resizing just
/// changes how much of the curve plays).
#[tauri::command]
pub fn resize_automation_clip(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    length_ticks: u64,
) -> Result<(), String> {
    with_clips(&state, &track_id, |clips| {
        let clip = clips
            .iter_mut()
            .find(|c| c.id == clip_id)
            .ok_or_else(|| format!("Automation clip not found: {clip_id}"))?;
        clip.length_ticks = length_ticks.max(1);
        Ok(())
    })
}

#[tauri::command]
pub fn add_automation_clip_point(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    tick: u64,
    value: f64,
) -> Result<(), String> {
    with_clips(&state, &track_id, |clips| {
        let clip = clips
            .iter_mut()
            .find(|c| c.id == clip_id)
            .ok_or_else(|| format!("Automation clip not found: {clip_id}"))?;
        clip.insert_point(tick, value.clamp(0.0, 1.0), CurveMode::Linear);
        Ok(())
    })
}

#[tauri::command]
pub fn move_automation_clip_point(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    point_index: usize,
    tick: u64,
    value: f64,
) -> Result<(), String> {
    with_clips(&state, &track_id, |clips| {
        let clip = clips
            .iter_mut()
            .find(|c| c.id == clip_id)
            .ok_or_else(|| format!("Automation clip not found: {clip_id}"))?;
        if !clip.move_point(point_index, tick, value.clamp(0.0, 1.0)) {
            return Err(format!("Point index out of range: {point_index}"));
        }
        Ok(())
    })
}

#[tauri::command]
pub fn remove_automation_clip_point(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    point_index: usize,
) -> Result<(), String> {
    with_clips(&state, &track_id, |clips| {
        let clip = clips
            .iter_mut()
            .find(|c| c.id == clip_id)
            .ok_or_else(|| format!("Automation clip not found: {clip_id}"))?;
        clip.remove_point(point_index);
        Ok(())
    })
}
