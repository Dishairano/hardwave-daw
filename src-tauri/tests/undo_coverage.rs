//! Every command that changes the song has to take an undo snapshot first.
//!
//! Undo works by snapshotting the project before a mutation. A command that
//! forgets leaves a hole: the edit happens, nothing is recorded, and the next
//! undo jumps further back than the person expects, past work they wanted to
//! keep. Four commands had that hole (the tempo, loading a sample into the
//! sampler, relinking a file, collecting samples) and nothing would have
//! caught the fifth.
//!
//! This reads the command sources and fails when a command writes project
//! state without snapshotting, unless it is listed below with a reason.

use std::collections::BTreeSet;
use std::path::Path;

/// Commands that write project state on purpose without an undo step.
///
/// Each one needs a reason. "It is awkward" is not a reason: the question is
/// whether a person would ever press Ctrl+Z expecting that change back.
const ALLOWED: &[(&str, &str)] = &[
    (
        "stems_service",
        "returns the service address; it changes nothing",
    ),
    (
        "set_modulation",
        "depth and centre are knobs; a sweep of one would fill the history",
    ),
    (
        "set_vca_gain",
        "a fader being moved; a whole move would fill the history",
    ),
    (
        "set_vca_muted",
        "a mute button on a group, the same as a track mute, which is also not a history step",
    ),
    (
        "set_macro_value",
        "turning a knob; a whole sweep would fill the history and make Ctrl+Z useless",
    ),
    (
        "apply_all_macros",
        "pushes saved macro values into the engine after a song opens; changes nothing",
    ),
    (
        "duplicate_section",
        "calls insert_time, which takes the snapshot; one Ctrl+Z undoes the whole repeat",
    ),
    (
        "save_project",
        "writing the file to disk changes nothing in the song",
    ),
    (
        "new_project",
        "replaces the whole project; the history is cleared with it",
    ),
    (
        "load_project",
        "replaces the whole project; the history is cleared with it",
    ),
    (
        "set_channel_rack_state",
        "the UI writes this blob on every step toggle; the rack owns its own undo",
    ),
    (
        "set_timeline_state",
        "the UI writes this blob continuously while dragging markers",
    ),
    (
        "set_automation_lane_visible",
        "showing or hiding a lane is a view change, not an edit to the song",
    ),
    (
        "clear_midi_capture",
        "empties the rolling input buffer, which is not part of the song",
    ),
    (
        "automation_write_sample",
        "one sample of a fader move; the pass takes its snapshot when it starts",
    ),
    (
        "commit_recording_to_midi_clip",
        "takes its own snapshot before the loop over passes",
    ),
];

/// Helpers that snapshot on the command's behalf.
const SNAPSHOTTING_HELPERS: &[&str] = &[
    "snapshot_before_mutation",
    "with_clip_notes(",
    "with_clips(",
    "with_track(",
    "mutate_project(",
];

/// Shapes that mean "this writes something the project saves".
///
/// Reading the same fields is everywhere, so each marker is a mutation:
/// a `_mut` accessor, a push or a removal, never a plain read.
fn writes_project_state(body: &str) -> bool {
    const MARKERS: &[&str] = &[
        "track_mut(",
        ".clips.push",
        ".clips.retain",
        ".clips.remove",
        ".clips.clear",
        "set_plugin_state",
        "automation_lanes.push",
        "automation_lanes.retain",
        "arrangements.push",
        "arrangements.retain",
        "entries.get_mut",
        "entries.iter_mut",
        "entries.push",
        "entries.retain",
    ];
    MARKERS.iter().any(|m| body.contains(m))
}

/// Commands that only read. Named by prefix, the way the codebase does.
fn is_a_getter(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "get_", "list_", "find_", "is_", "has_", "read_", "dump_", "peek_",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
}

fn command_bodies(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for chunk in src.split("#[tauri::command]").skip(1) {
        let Some(idx) = chunk.find("fn ") else {
            continue;
        };
        let rest = &chunk[idx + 3..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        out.push((name, chunk.to_string()));
    }
    out
}

#[test]
fn every_command_that_edits_the_song_takes_an_undo_snapshot() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    let allowed: BTreeSet<&str> = ALLOWED.iter().map(|(name, _)| *name).collect();

    let mut missing: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&dir).expect("commands dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read command file");
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        for (name, body) in command_bodies(&src) {
            checked += 1;
            if allowed.contains(name.as_str()) || is_a_getter(&name) {
                continue;
            }
            if SNAPSHOTTING_HELPERS.iter().any(|h| body.contains(h)) {
                continue;
            }
            if writes_project_state(&body) {
                missing.push(format!("{file}::{name}"));
            }
        }
    }

    assert!(
        checked > 100,
        "the scanner found only {checked} commands, so it is not reading the sources any more"
    );
    assert!(
        missing.is_empty(),
        "these commands change the song without an undo snapshot:\n  {}\n\
         Add `state.engine.lock().snapshot_before_mutation();` before the edit, \
         or list the command in ALLOWED with the reason it needs no undo step.",
        missing.join("\n  ")
    );
}

#[test]
fn the_allow_list_has_no_stale_entries() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    let mut names: BTreeSet<String> = BTreeSet::new();
    for entry in std::fs::read_dir(&dir).expect("commands dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read command file");
        for (name, _) in command_bodies(&src) {
            names.insert(name);
        }
    }
    let stale: Vec<&str> = ALLOWED
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !names.contains(*n))
        .collect();
    assert!(
        stale.is_empty(),
        "the allow list names commands that no longer exist: {stale:?}"
    );
}
