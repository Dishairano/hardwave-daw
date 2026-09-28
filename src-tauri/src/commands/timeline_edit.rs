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

/// Sections live in the same blob and name a range, so both ends move.
///
/// The end of a section that stops exactly where room is being made stays
/// put: the new bars come after that section, not inside it. When time is
/// being taken out the same edge does move, because the stretch that is
/// disappearing sits in front of it.
fn shift_sections(project: &mut hardwave_project::Project, at: u64, delta: i64) {
    let Some(raw) = project.timeline_state.as_deref() else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return;
    };
    let Some(sections) = value.get_mut(SECTION_KEY).and_then(|s| s.as_array_mut()) else {
        return;
    };
    for section in sections.iter_mut() {
        let start = section.get("startTicks").and_then(|t| t.as_u64());
        let end = section.get("endTicks").and_then(|t| t.as_u64());
        let (Some(start), Some(end)) = (start, end) else {
            continue;
        };
        let new_start = if start >= at {
            apply(start, delta)
        } else {
            start
        };
        let end_moves = if delta > 0 { end > at } else { end >= at };
        let new_end = if end_moves { apply(end, delta) } else { end };
        section["startTicks"] = serde_json::json!(new_start);
        section["endTicks"] = serde_json::json!(new_end.max(new_start));
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
        shift_sections(&mut project, from, delta);
    }

    // The graph rebuild is what re-reads the tempo map and the clips, which
    // is the same path every other timeline edit uses.
    state.engine.lock().rebuild_graph();
    Ok(())
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

/// A named stretch of the song: verse, chorus, drop.
///
/// Arranging meant selecting clips across every track and hoping the
/// selection was right. A section names a range once, and duplicating it
/// makes room and copies everything inside, which is what "repeat the
/// chorus" actually means.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub id: String,
    pub name: String,
    pub start_ticks: u64,
    pub end_ticks: u64,
}

/// Sections ride in the project's timeline blob, beside the markers.
const SECTION_KEY: &str = "sections";

fn read_sections(project: &hardwave_project::Project) -> Vec<Section> {
    project
        .timeline_state
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|v| v.get(SECTION_KEY).cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn write_sections(project: &mut hardwave_project::Project, sections: &[Section]) {
    let mut value: serde_json::Value = project
        .timeline_state
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !value.is_object() {
        value = serde_json::json!({});
    }
    value[SECTION_KEY] = serde_json::to_value(sections).unwrap_or(serde_json::Value::Null);
    project.timeline_state = Some(value.to_string());
}

#[tauri::command]
pub fn list_sections(state: State<AppState>) -> Vec<Section> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    let mut sections = read_sections(&project);
    sections.sort_by_key(|s| s.start_ticks);
    sections
}

#[tauri::command]
pub fn add_section(
    state: State<AppState>,
    name: String,
    start_ticks: u64,
    end_ticks: u64,
) -> Result<String, String> {
    if end_ticks <= start_ticks {
        return Err("a section needs some length".into());
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("give the section a name".into());
    }
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let mut sections = read_sections(&project);
    let id = uuid::Uuid::new_v4().to_string();
    sections.push(Section {
        id: id.clone(),
        name,
        start_ticks,
        end_ticks,
    });
    write_sections(&mut project, &sections);
    Ok(id)
}

#[tauri::command]
pub fn delete_section(state: State<AppState>, id: String) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let mut sections = read_sections(&project);
    let before = sections.len();
    sections.retain(|s| s.id != id);
    if sections.len() == before {
        return Err("no section with that id".into());
    }
    write_sections(&mut project, &sections);
    Ok(())
}

/// Repeat a section: make room straight after it and copy everything inside
/// into the new space.
///
/// Automation and markers move with the insert, because that is what
/// `insert_time` does; what is copied is the clips, which is what a repeat
/// of a chorus is made of.
#[tauri::command]
pub fn duplicate_section(state: State<AppState>, id: String) -> Result<usize, String> {
    let section = {
        let engine = state.engine.lock();
        let project = engine.project.lock();
        read_sections(&project)
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| "no section with that id".to_string())?
    };
    let length = section.end_ticks.saturating_sub(section.start_ticks);
    if length == 0 {
        return Err("that section has no length".into());
    }

    // Make the room first. This takes its own undo snapshot, and the copy
    // below rides in the same step because nothing snapshots in between.
    insert_time(state.clone(), section.end_ticks, length)?;

    let copied = {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let mut copied = 0usize;
        for track in project.tracks.iter_mut() {
            let originals: Vec<hardwave_project::clip::ClipPlacement> = track
                .clips
                .iter()
                .filter(|c| {
                    c.position_ticks >= section.start_ticks
                        && c.position_ticks + c.length_ticks <= section.end_ticks
                })
                .cloned()
                .collect();
            for original in originals {
                let mut copy = original.clone();
                copy.position_ticks += length;
                // A copy is its own clip: sharing an id would make the two
                // impossible to edit apart.
                match &mut copy.content {
                    ClipContent::Audio(ac) => ac.id = uuid::Uuid::new_v4().to_string(),
                    ClipContent::Midi(mc) => {
                        let fresh = uuid::Uuid::new_v4().to_string();
                        mc.id = fresh.clone();
                        mc.clip.id = fresh;
                    }
                }
                track.clips.push(copy);
                copied += 1;
            }
        }
        // The section itself repeats too, so the copy can be worked with.
        let mut sections = read_sections(&project);
        sections.push(Section {
            id: uuid::Uuid::new_v4().to_string(),
            name: format!("{} copy", section.name),
            start_ticks: section.end_ticks,
            end_ticks: section.end_ticks + length,
        });
        write_sections(&mut project, &sections);
        copied
    };

    state.engine.lock().rebuild_graph();
    Ok(copied)
}

#[cfg(test)]
mod section_shift_tests {
    use super::*;

    fn project_with(blob: &str) -> hardwave_project::Project {
        hardwave_project::Project {
            timeline_state: Some(blob.to_string()),
            ..Default::default()
        }
    }

    fn sections_of(project: &hardwave_project::Project) -> Vec<Section> {
        read_sections(project)
    }

    #[test]
    fn making_room_after_a_section_leaves_it_alone() {
        let mut project = project_with(
            r#"{"sections":[{"id":"a","name":"Drop","startTicks":1920,"endTicks":3840}]}"#,
        );
        shift_sections(&mut project, 3840, 1920);
        let sections = sections_of(&project);
        assert_eq!(sections[0].start_ticks, 1920);
        assert_eq!(sections[0].end_ticks, 3840);
    }

    #[test]
    fn making_room_before_a_section_moves_the_whole_section() {
        let mut project = project_with(
            r#"{"sections":[{"id":"a","name":"Drop","startTicks":1920,"endTicks":3840}]}"#,
        );
        shift_sections(&mut project, 0, 960);
        let sections = sections_of(&project);
        assert_eq!(sections[0].start_ticks, 2880);
        assert_eq!(sections[0].end_ticks, 4800);
    }

    #[test]
    fn making_room_inside_a_section_stretches_it() {
        let mut project = project_with(
            r#"{"sections":[{"id":"a","name":"Drop","startTicks":1920,"endTicks":3840}]}"#,
        );
        shift_sections(&mut project, 2880, 960);
        let sections = sections_of(&project);
        assert_eq!(sections[0].start_ticks, 1920);
        assert_eq!(sections[0].end_ticks, 4800);
    }

    #[test]
    fn taking_time_out_in_front_pulls_the_section_back() {
        let mut project = project_with(
            r#"{"sections":[{"id":"a","name":"Drop","startTicks":1920,"endTicks":3840}]}"#,
        );
        // delete_time shifts from the end of the deleted stretch.
        shift_sections(&mut project, 1920, -960);
        let sections = sections_of(&project);
        assert_eq!(sections[0].start_ticks, 960);
        assert_eq!(sections[0].end_ticks, 2880);
    }

    #[test]
    fn a_blob_without_sections_is_left_exactly_as_it_was() {
        let mut project = project_with(r#"{"markers":[{"id":"m","tick":480}]}"#);
        shift_sections(&mut project, 0, 960);
        assert_eq!(
            project.timeline_state.as_deref(),
            Some(r#"{"markers":[{"id":"m","tick":480}]}"#)
        );
    }
}
