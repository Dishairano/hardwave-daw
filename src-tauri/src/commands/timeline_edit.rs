//! Insert and delete stretches of time across the whole song.
//!
//! Adding two bars in the middle of an arrangement meant selecting
//! everything after that point on every track and dragging it, hoping
//! nothing slipped and remembering that automation and markers do not come
//! along when you drag clips. Deleting a section had the same problem in
//! reverse. Both are one command here, applied to every track at once:
//! clips, automation lanes, automation clips, tempo and time-signature
//! changes, and the markers on the ruler.

use crate::AppState;
use hardwave_project::clip::ClipContent;
use tauri::State;

/// Shift one clip placement, and the notes or controls inside a MIDI clip
/// are untouched because they are stored relative to the clip.
fn shift_clip(placement: &mut hardwave_project::clip::ClipPlacement, delta: i64) {
    placement.position_ticks = apply(placement.position_ticks, delta);
}

fn apply(tick: u64, delta: i64) -> u64 {
    if delta >= 0 {
        tick.saturating_add(delta as u64)
    } else {
        tick.saturating_sub((-delta) as u64)
    }
}

/// Markers live in an opaque JSON blob the UI owns, so they are shifted by
/// walking the JSON rather than by a typed model. A blob that does not look
/// like markers is left exactly as it was.
fn shift_markers(project: &mut hardwave_project::Project, at: u64, delta: i64) {
    let Some(raw) = project.timeline_state.as_deref() else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return;
    };
    let Some(markers) = value.get_mut("markers").and_then(|m| m.as_array_mut()) else {
        return;
    };
    for marker in markers.iter_mut() {
        let Some(tick) = marker.get("tick").and_then(|t| t.as_u64()) else {
            continue;
        };
        if tick < at {
            continue;
        }
        if let Some(slot) = marker.get_mut("tick") {
            *slot = serde_json::json!(apply(tick, delta));
        }
    }
    project.timeline_state = Some(value.to_string());
}

/// Make room: everything at or after `at_ticks` moves later by `length_ticks`.
///
/// A clip that straddles the point is moved rather than split. Splitting it
/// would be a guess about what the person wanted; moving it keeps the audio
/// intact and is undone with one Ctrl+Z.
#[tauri::command]
pub fn insert_time(state: State<AppState>, at_ticks: u64, length_ticks: u64) -> Result<(), String> {
    if length_ticks == 0 {
        return Err("choose a length to insert".into());
    }
    shift_everything(state, at_ticks, length_ticks as i64)
}

/// Take a stretch out: everything after `at_ticks + length_ticks` moves
/// earlier, and anything that lies wholly inside the stretch is removed.
///
/// A clip that only overlaps the edge is moved, not trimmed: cutting into
/// someone's audio without being asked is worse than leaving it whole.
#[tauri::command]
pub fn delete_time(state: State<AppState>, at_ticks: u64, length_ticks: u64) -> Result<(), String> {
    if length_ticks == 0 {
        return Err("choose a length to delete".into());
    }
    let end = at_ticks.saturating_add(length_ticks);
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        for track in project.tracks.iter_mut() {
            track.clips.retain(|c| {
                let clip_end = c.position_ticks + c.length_ticks;
                !(c.position_ticks >= at_ticks && clip_end <= end)
            });
            for lane in track.automation_lanes.iter_mut() {
                lane.points.retain(|p| p.tick < at_ticks || p.tick >= end);
            }
            track
                .automation_clips
                .retain(|c| !(c.start_tick >= at_ticks && c.start_tick + c.length_ticks <= end));
        }
    }
    shift_everything(state, end, -(length_ticks as i64))
}

/// Move everything at or after `from` by `delta` ticks.
fn shift_everything(state: State<AppState>, from: u64, delta: i64) -> Result<(), String> {
    if delta > 0 {
        // insert_time has not taken its snapshot yet; delete_time has.
        state.engine.lock().snapshot_before_mutation();
    }
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();

        for track in project.tracks.iter_mut() {
            for placement in track.clips.iter_mut() {
                if placement.position_ticks >= from {
                    shift_clip(placement, delta);
                }
                // A MIDI clip carries its id twice; nothing inside it moves,
                // because notes and controls are relative to the clip.
                if let ClipContent::Midi(_) = placement.content {
                    continue;
                }
            }
            for lane in track.automation_lanes.iter_mut() {
                for point in lane.points.iter_mut() {
                    if point.tick >= from {
                        point.tick = apply(point.tick, delta);
                    }
                }
            }
            for clip in track.automation_clips.iter_mut() {
                if clip.start_tick >= from {
                    clip.start_tick = apply(clip.start_tick, delta);
                }
            }
        }

        // Tempo and time-signature changes are part of the arrangement: a
        // key change two bars later has to stay two bars later. The entry at
        // tick 0 is the song's starting tempo and never moves.
        for entry in project.tempo_map.entries.iter_mut() {
            if entry.tick >= from && entry.tick > 0 {
                entry.tick = apply(entry.tick, delta);
            }
        }

        shift_markers(&mut project, from, delta);
    }

    // The graph rebuild is what re-reads the tempo map and the clips, which
    // is the same path every other timeline edit uses.
    state.engine.lock().rebuild_graph();
    Ok(())
}
