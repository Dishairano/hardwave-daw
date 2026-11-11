//! The playlist's rows: FL's playlist tracks.
//!
//! Each edit is one undo step and changes only the song, so a room carries
//! it with no work of its own: the rows travel as the song's "rows" part
//! and a clip's row inside its track. Where a clip sits never changes how
//! it sounds; only a row's mute and solo do, so those rebuild the graph.

use hardwave_engine::DawEngine;
use hardwave_project::{PlaylistRow, Project};
use serde::Deserialize;
use tauri::State;

use crate::commands::audio::ImportedClip;
use crate::AppState;

/// One edit: checked first, so a refused one leaves no undo step behind,
/// then a snapshot for undo, the change, and a graph rebuild when it can
/// change what plays.
fn edit<T>(
    engine: &DawEngine,
    rebuild: bool,
    check: impl FnOnce(&Project) -> Result<(), String>,
    apply: impl FnOnce(&mut Project) -> Result<T, String>,
) -> Result<T, String> {
    check(&engine.project.lock())?;
    engine.snapshot_before_mutation();
    let out = apply(&mut engine.project.lock());
    if rebuild {
        engine.rebuild_graph();
    }
    out
}

fn no_check(_: &Project) -> Result<(), String> {
    Ok(())
}

/// The open arrangement's rows from the top. A row past the end of the
/// list is an unnamed one with its switches on.
#[tauri::command]
pub fn get_playlist_rows(state: State<AppState>) -> Vec<PlaylistRow> {
    state.engine.lock().project.lock().rows.clone()
}

/// A clip to put on a row.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ClipRow {
    pub track_id: String,
    pub clip_id: String,
    pub row: u32,
}

fn set_clip_rows_on(engine: &DawEngine, moves: Vec<ClipRow>) -> Result<usize, String> {
    let moves: Vec<(String, String, u32)> = moves
        .into_iter()
        .map(|m| (m.track_id, m.clip_id, m.row))
        .collect();
    let check = |p: &Project| p.check_clip_rows(&moves);
    edit(engine, true, check, |p| p.set_clip_rows(&moves))
}

/// Move clips to other rows; they stay on their channels and sound the
/// same. Returns how many moved.
#[tauri::command]
pub fn set_clip_rows(state: State<AppState>, moves: Vec<ClipRow>) -> Result<usize, String> {
    set_clip_rows_on(&state.engine.lock(), moves)
}

fn set_automation_clip_row_on(
    engine: &DawEngine,
    track_id: String,
    clip_id: String,
    row: u32,
) -> Result<(), String> {
    let check = |p: &Project| {
        let found = p
            .track(&track_id)
            .is_some_and(|t| t.automation_clips.iter().any(|c| c.id == clip_id));
        if found {
            Ok(())
        } else {
            Err(format!("Automation clip not found: {clip_id}"))
        }
    };
    edit(engine, true, check, |p| {
        p.set_automation_clip_row(&track_id, &clip_id, row)
    })
}

/// Move an automation clip to another row; it moves the same knob.
#[tauri::command]
pub fn set_automation_clip_row(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    row: u32,
) -> Result<(), String> {
    set_automation_clip_row_on(&state.engine.lock(), track_id, clip_id, row)
}

/// Name a row; an empty name reads "Track N" again.
#[tauri::command]
pub fn set_row_name(state: State<AppState>, row: u32, name: String) -> Result<(), String> {
    edit(&state.engine.lock(), false, no_check, |p| {
        p.set_row_name(row, &name)
    })
}

/// Colour a row (`#rrggbb`), or give it the default back with none.
#[tauri::command]
pub fn set_row_color(
    state: State<AppState>,
    row: u32,
    color: Option<String>,
) -> Result<(), String> {
    edit(&state.engine.lock(), false, no_check, |p| {
        p.set_row_color(row, color.as_deref())
    })
}

/// FL's track switch: the row's clips go quiet (or play again).
#[tauri::command]
pub fn set_row_mute(state: State<AppState>, row: u32, muted: bool) -> Result<(), String> {
    edit(&state.engine.lock(), true, no_check, |p| {
        p.set_row_mute(row, muted)
    })
}

/// Solo a row: while any row is soloed only soloed rows play.
#[tauri::command]
pub fn set_row_solo(state: State<AppState>, row: u32, soloed: bool) -> Result<(), String> {
    edit(&state.engine.lock(), true, no_check, |p| {
        p.set_row_solo(row, soloed)
    })
}

/// Put empty rows in at `at`; everything under it moves down.
#[tauri::command]
pub fn insert_rows(state: State<AppState>, at: u32, count: u32) -> Result<(), String> {
    edit(&state.engine.lock(), true, no_check, |p| {
        p.insert_rows(at, count)
    })
}

/// Take rows out with the clips on them; the rows under them move up.
#[tauri::command]
pub fn delete_rows(state: State<AppState>, at: u32, count: u32) -> Result<(), String> {
    edit(&state.engine.lock(), true, no_check, |p| {
        p.delete_rows(at, count)
    })
}

fn place_clip_copy_on(
    engine: &DawEngine,
    track_id: String,
    clip_id: String,
    position_ticks: u64,
    row: u32,
) -> Result<String, String> {
    let moves = vec![(track_id.clone(), clip_id.clone(), row)];
    let check = |p: &Project| p.check_clip_rows(&moves);
    edit(engine, true, check, |p| {
        p.place_clip_copy(
            &track_id,
            &clip_id,
            position_ticks,
            row,
            uuid::Uuid::new_v4().to_string(),
        )
    })
}

/// The picker's place: a copy of a clip on its own channel, at a tick on
/// the row under the mouse. Returns the copy's id.
#[tauri::command]
pub fn place_clip_copy(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    position_ticks: u64,
    row: u32,
) -> Result<String, String> {
    place_clip_copy_on(&state.engine.lock(), track_id, clip_id, position_ticks, row)
}

/// A sample put on the playlist and the channel it plays through.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacedSample {
    pub channel_id: String,
    /// The channel was made for this sample now.
    pub new_channel: bool,
    pub clip: ImportedClip,
}

pub(crate) fn place_sample_on(
    engine: &DawEngine,
    file_path: String,
    row: u32,
    position_ticks: u64,
) -> Result<PlacedSample, String> {
    hardwave_project::row_edit::check_row_number(row)?;
    let name = std::path::Path::new(&file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Sample")
        .to_string();
    engine.snapshot_before_mutation();
    let (channel_id, new_channel, renamed) = {
        let mut project = engine.project.lock();
        let before: Vec<(String, String)> = project
            .tracks
            .iter()
            .map(|t| (t.id.clone(), t.name.clone()))
            .collect();
        let (id, made) =
            project.sample_channel(&file_path, &name, uuid::Uuid::new_v4().to_string());
        let renamed: Vec<(String, String)> = before
            .into_iter()
            .filter(|(tid, old)| project.track(tid).is_some_and(|t| &t.name != old))
            .collect();
        (id, made, renamed)
    };
    let placed = crate::commands::audio::place_audio_file(
        engine,
        channel_id.clone(),
        file_path,
        Some(position_ticks),
    );
    let clip = match placed {
        Ok(clip) => clip,
        Err(e) => {
            // The file did not load: the song goes back to how it was.
            let mut project = engine.project.lock();
            if new_channel {
                project.tracks.retain(|t| t.id != channel_id);
            }
            for (id, old) in renamed {
                if let Some(t) = project.track_mut(&id) {
                    t.name = old;
                }
            }
            drop(project);
            engine.rebuild_graph();
            return Err(e);
        }
    };
    {
        let mut project = engine.project.lock();
        project.set_clip_rows(&[(channel_id.clone(), clip.clip_id.clone(), row)])?;
    }
    engine.sync_track_meters();
    engine.rebuild_graph();
    Ok(PlacedSample {
        channel_id,
        new_channel,
        clip,
    })
}

/// A sample dropped on the playlist: its clip on the row under the mouse,
/// on the sample's own audio clip channel (made the first time, playing
/// into the next free insert). One undo step. Off the window's thread, as
/// decoding a long file there froze the window.
#[tauri::command]
pub async fn place_sample_on_row(
    app: tauri::AppHandle,
    file_path: String,
    row: u32,
    position_ticks: u64,
) -> Result<PlacedSample, String> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        // Picked by the person, so a network server it is on is one they
        // use for samples (as for an import).
        if hardwave_engine::source_paths::network_server(&file_path).is_some() {
            state.engine.lock().trust_network_location(&file_path);
            crate::commands::engine::persist_audio_prefs_public(&state);
        }
        let engine = state.engine.lock();
        place_sample_on(&engine, file_path, row, position_ticks)
    })
    .await
    .map_err(|e| format!("placing the sample stopped: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};
    use hardwave_project::row_edit::clip_id;

    const PPQ: u64 = 960;

    /// A clip of a held note on the built-in sine, two beats long.
    fn note_clip(track: &str, id: &str, at: u64, pitch: u8, row: u32) -> ClipPlacement {
        let mut clip = hardwave_midi::MidiClip::new(id.into(), id.into(), 2 * PPQ);
        clip.notes.push(hardwave_midi::MidiNote {
            start_tick: 0,
            duration_ticks: 2 * PPQ,
            pitch,
            velocity: 0.8,
            channel: 0,
            muted: false,
            pan: 0.0,
            fine_cents: 0.0,
            release_velocity: 0.5,
        });
        ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: id.into(),
                clip,
            }),
            track_id: track.into(),
            position_ticks: at,
            length_ticks: 2 * PPQ,
            lane: 0,
            color: None,
            row: Some(row),
        }
    }

    /// A new song with a lead (one clip on row 0) and a bass (one clip on
    /// row 1), both on the built-in sine, laid out as a song made here.
    fn engine() -> DawEngine {
        let engine = DawEngine::new();
        {
            let mut p = engine.project.lock();
            *p = Project::default();
            let lead = p.claim_row(1, "Lead", true);
            let bass = p.claim_row(2, "Bass", true);
            p.track_mut(&lead)
                .unwrap()
                .clips
                .push(note_clip(&lead, "lead-1", 0, 72, 0));
            p.track_mut(&bass)
                .unwrap()
                .clips
                .push(note_clip(&bass, "bass-1", 0, 40, 1));
        }
        engine.rebuild_graph();
        engine
    }

    fn render(engine: &DawEngine) -> Vec<f32> {
        let mut out = Vec::new();
        engine
            .render_offline(48_000, 48_000, |block| {
                out.extend_from_slice(block);
                true
            })
            .expect("renders");
        out
    }

    fn row_of(engine: &DawEngine, track: &str, id: &str) -> Option<u32> {
        engine
            .project
            .lock()
            .track(track)
            .and_then(|t| t.clips.iter().find(|c| clip_id(c) == id))
            .and_then(|c| c.row)
    }

    /// The edit happened, undo takes it back, and redo brings it again.
    fn undoes(engine: &DawEngine, look: impl Fn(&Project) -> String, before: &str, after: &str) {
        assert_eq!(look(&engine.project.lock()), after, "the edit");
        assert!(engine.undo());
        assert_eq!(look(&engine.project.lock()), before, "undone");
        assert!(engine.redo());
        assert_eq!(look(&engine.project.lock()), after, "redone");
    }

    /// What the other person in a room ends with after this side's edit:
    /// the song parts and the tracks that changed, taken as a room takes
    /// them.
    fn in_a_room(engine: &DawEngine, edit: impl FnOnce(&DawEngine)) -> Project {
        let start = engine.project.lock().clone();
        let collab = crate::collab::Collab::default();
        collab.remember_tracks(&start);
        edit(engine);
        let now = engine.project.lock().clone();
        let mut theirs = start;
        for (name, _, _, blob) in collab.song_changes(&now) {
            theirs.put_song_section(name, &blob).unwrap();
        }
        let (changed, gone) = collab.track_changes(&now);
        for (_, index, blob) in changed {
            theirs.put_track_blob(&blob, index).unwrap();
        }
        theirs.tracks.retain(|t| !gone.contains(&t.id));
        theirs
    }

    /// A track's id and the rows of its clips and automation clips.
    type TrackRows = (String, Vec<Option<u32>>, Vec<Option<u32>>);

    fn same_layout(a: &Project, b: &Project) {
        assert_eq!(a.rows, b.rows, "the rows");
        let rows = |p: &Project| -> Vec<TrackRows> {
            p.tracks
                .iter()
                .map(|t| {
                    (
                        t.id.clone(),
                        t.clips.iter().map(|c| c.row).collect(),
                        t.automation_clips.iter().map(|c| c.row).collect(),
                    )
                })
                .collect()
        };
        assert_eq!(rows(a), rows(b), "every clip's row");
    }

    #[test]
    fn moving_clips_to_other_rows_is_one_undo_step_and_reaches_the_room() {
        let e = engine();
        let theirs = in_a_room(&e, |e| {
            let moves = vec![ClipRow {
                track_id: "insert-001".into(),
                clip_id: "lead-1".into(),
                row: 7,
            }];
            assert_eq!(set_clip_rows_on(e, moves).unwrap(), 1);
        });
        same_layout(&e.project.lock(), &theirs);
        let look = |p: &Project| format!("{:?}", p.track("insert-001").unwrap().clips[0].row);
        undoes(&e, look, "Some(0)", "Some(7)");
        // A clip that is not there: refused, and no undo step left.
        let (steps, _) = e.history_sizes();
        let bad = vec![ClipRow {
            track_id: "insert-001".into(),
            clip_id: "nope".into(),
            row: 1,
        }];
        assert!(set_clip_rows_on(&e, bad).is_err());
        assert_eq!(e.history_sizes().0, steps);
    }

    #[test]
    fn a_clip_made_without_a_row_stays_where_it_is_drawn_through_row_edits() {
        let e = engine();
        // As a recording or an import leaves it: no row, so it is drawn
        // with the lead's other clip, on row 0.
        let mut late = note_clip("insert-001", "lead-2", 4 * PPQ, 72, 0);
        late.row = None;
        e.project
            .lock()
            .track_mut("insert-001")
            .unwrap()
            .clips
            .push(late);
        let theirs = in_a_room(&e, |e| {
            let moves = vec![ClipRow {
                track_id: "insert-001".into(),
                clip_id: "lead-1".into(),
                row: 7,
            }];
            set_clip_rows_on(e, moves).unwrap();
        });
        assert_eq!(
            row_of(&e, "insert-001", "lead-2"),
            Some(0),
            "it stays on its row and does not follow the clip that moved"
        );
        same_layout(&e.project.lock(), &theirs);
        edit(&e, true, no_check, |p| p.insert_rows(0, 2)).unwrap();
        assert_eq!(row_of(&e, "insert-001", "lead-2"), Some(2), "moved down");
        assert!(e.undo() && e.undo());
        assert_eq!(row_of(&e, "insert-001", "lead-1"), Some(0));
        assert_eq!(
            row_of(&e, "insert-001", "lead-2"),
            Some(0),
            "undone, it is still where it was drawn"
        );
    }

    #[test]
    fn a_row_drag_changes_nothing_in_the_sound() {
        let e = engine();
        let before = render(&e);
        assert!(before.iter().any(|s| s.abs() > 0.05), "the song plays");
        set_clip_rows_on(
            &e,
            vec![
                ClipRow {
                    track_id: "insert-001".into(),
                    clip_id: "lead-1".into(),
                    row: 3,
                },
                ClipRow {
                    track_id: "insert-002".into(),
                    clip_id: "bass-1".into(),
                    row: 0,
                },
            ],
        )
        .unwrap();
        assert_eq!(row_of(&e, "insert-001", "lead-1"), Some(3));
        let after = render(&e);
        assert_eq!(before.len(), after.len());
        let differs = before
            .iter()
            .zip(&after)
            .position(|(a, b)| a.to_bits() != b.to_bits());
        assert_eq!(differs, None, "the same sound, sample for sample");
    }

    #[test]
    fn a_muted_row_silences_its_clips_and_a_soloed_row_the_others() {
        let e = engine();
        let both = render(&e);
        // The lead alone, for comparing.
        let lead_only = {
            let solo = engine();
            solo.project
                .lock()
                .track_mut("insert-002")
                .unwrap()
                .clips
                .clear();
            solo.rebuild_graph();
            render(&solo)
        };
        let row = |e: &DawEngine, r: u32| {
            edit(e, true, no_check, |p| p.set_row_mute(r, true)).unwrap();
        };
        row(&e, 1);
        let muted = render(&e);
        assert_ne!(muted, both, "the bass went quiet");
        assert_eq!(muted, lead_only, "only the lead plays");
        let look = |p: &Project| format!("{}", p.rows[1].muted);
        undoes(&e, look, "false", "true");
        assert!(e.undo());
        e.rebuild_graph();
        // Solo row 0: the bass on row 1 is silent again.
        edit(&e, true, no_check, |p| p.set_row_solo(0, true)).unwrap();
        assert_eq!(render(&e), lead_only, "only the soloed row plays");
        // Moving the bass onto the soloed row brings it back.
        set_clip_rows_on(
            &e,
            vec![ClipRow {
                track_id: "insert-002".into(),
                clip_id: "bass-1".into(),
                row: 0,
            }],
        )
        .unwrap();
        assert_eq!(render(&e), both, "both play, as before any switch");
    }

    #[test]
    fn row_names_colours_and_switches_undo_and_reach_the_room() {
        let e = engine();
        let theirs = in_a_room(&e, |e| {
            edit(e, false, no_check, |p| p.set_row_name(4, "Drums")).unwrap();
            edit(e, false, no_check, |p| p.set_row_color(4, Some("#112233"))).unwrap();
            edit(e, true, no_check, |p| p.set_row_mute(2, true)).unwrap();
            edit(e, true, no_check, |p| p.set_row_solo(3, true)).unwrap();
        });
        same_layout(&e.project.lock(), &theirs);
        assert_eq!(theirs.rows[4].name, "Drums");
        let look = |p: &Project| format!("{}", p.rows[3].soloed);
        undoes(&e, look, "false", "true");
        assert!(e.undo(), "the solo again");
        let look = |p: &Project| format!("{}", p.rows[2].muted);
        assert!(e.undo());
        assert_eq!(look(&e.project.lock()), "false", "mute undone");
        let look = |p: &Project| format!("{:?}", p.rows[4].color);
        assert!(e.undo());
        assert_eq!(
            look(&e.project.lock()),
            "Some(\"#7c3aed\")",
            "colour undone: the track colour it was laid out with"
        );
        let look = |p: &Project| p.rows[4].name.clone();
        assert!(e.undo());
        assert_eq!(look(&e.project.lock()), "Insert 5", "name undone");
    }

    #[test]
    fn inserted_and_deleted_rows_undo_and_reach_the_room() {
        let e = engine();
        let theirs = in_a_room(&e, |e| {
            edit(e, true, no_check, |p| p.insert_rows(0, 2)).unwrap();
        });
        same_layout(&e.project.lock(), &theirs);
        assert_eq!(row_of(&e, "insert-002", "bass-1"), Some(3));
        let look = |p: &Project| format!("{:?}", p.track("insert-002").unwrap().clips[0].row);
        undoes(&e, look, "Some(1)", "Some(3)");

        let theirs = in_a_room(&e, |e| {
            edit(e, true, no_check, |p| p.delete_rows(2, 1)).unwrap();
        });
        same_layout(&e.project.lock(), &theirs);
        assert!(e
            .project
            .lock()
            .track("insert-001")
            .unwrap()
            .clips
            .is_empty());
        let look = |p: &Project| format!("{}", p.track("insert-001").unwrap().clips.len());
        undoes(&e, look, "1", "0");
    }

    #[test]
    fn an_automation_clip_moves_row_with_undo_and_reaches_the_room() {
        let e = engine();
        e.project
            .lock()
            .track_mut("insert-001")
            .unwrap()
            .automation_clips
            .push(hardwave_project::automation_clip::AutomationClip::new(
                "ac",
                hardwave_project::AutomationTarget::TrackVolume,
                0,
                PPQ,
            ));
        let theirs = in_a_room(&e, |e| {
            set_automation_clip_row_on(e, "insert-001".into(), "ac".into(), 12).unwrap();
        });
        same_layout(&e.project.lock(), &theirs);
        let look = |p: &Project| {
            format!(
                "{:?}",
                p.track("insert-001").unwrap().automation_clips[0].row
            )
        };
        // Made with no row, it was drawn on the first empty row (2); the
        // edit wrote that down first, so undo puts it back there.
        undoes(&e, look, "Some(2)", "Some(12)");
        assert!(set_automation_clip_row_on(&e, "insert-001".into(), "zz".into(), 1).is_err());
    }

    #[test]
    fn a_placed_copy_stays_on_its_channel_with_undo_and_reaches_the_room() {
        let e = engine();
        let mut id = String::new();
        let theirs = in_a_room(&e, |e| {
            id = place_clip_copy_on(e, "insert-002".into(), "bass-1".into(), 4 * PPQ, 9).unwrap();
        });
        same_layout(&e.project.lock(), &theirs);
        assert_eq!(row_of(&e, "insert-002", &id), Some(9));
        let look = |p: &Project| format!("{}", p.track("insert-002").unwrap().clips.len());
        undoes(&e, look, "1", "2");
    }

    #[test]
    fn a_dropped_sample_gets_its_channel_and_its_clip_on_the_row_in_one_step() {
        let dir = std::env::temp_dir().join(format!("hw-rows-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("kick.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&file, spec).unwrap();
        for i in 0..4800 {
            w.write_sample(((i as f32 * 0.05).sin() * 12_000.0) as i16)
                .unwrap();
        }
        w.finalize().unwrap();
        let path = file.to_string_lossy().into_owned();

        let e = engine();
        let (steps, _) = e.history_sizes();
        let mut placed = None;
        let theirs = in_a_room(&e, |e| {
            placed = Some(place_sample_on(e, path.clone(), 5, PPQ).unwrap());
        });
        let placed = placed.unwrap();
        assert_eq!(e.history_sizes().0, steps + 1, "one undo step");
        assert!(placed.new_channel);
        {
            let p = e.project.lock();
            let channel = p.track(&placed.channel_id).unwrap();
            assert_eq!(channel.name, "kick");
            assert_eq!(channel.rack_group, Some(0));
            // Inserts 1 and 2 are the lead and the bass; 3 is free.
            assert_eq!(channel.output_bus.as_deref(), Some("insert-003"));
            assert_eq!(p.track("insert-003").unwrap().name, "kick");
            assert_eq!(channel.clips[0].row, Some(5));
            assert_eq!(channel.clips[0].position_ticks, PPQ);
            same_layout(&p, &theirs);
        }
        // The same file again: the same channel.
        let again = place_sample_on(&e, path.clone(), 6, 0).unwrap();
        assert_eq!(again.channel_id, placed.channel_id);
        assert!(!again.new_channel);
        // Undo takes the second clip, then the channel with the first.
        assert!(e.undo() && e.undo());
        let p = e.project.lock();
        assert!(p.track(&placed.channel_id).is_none());
        assert_eq!(p.track("insert-003").unwrap().name, "Insert 3");
        drop(p);
        // A file that is not audio: refused, and the song as it was.
        let bad = dir.join("notes.wav");
        std::fs::write(&bad, b"not audio").unwrap();
        let tracks = e.project.lock().tracks.len();
        assert!(place_sample_on(&e, bad.to_string_lossy().into_owned(), 0, 0).is_err());
        let p = e.project.lock();
        assert_eq!(p.tracks.len(), tracks);
        assert_eq!(p.track("insert-003").unwrap().name, "Insert 3");
        drop(p);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_room_between_a_build_from_before_and_this_one_ends_with_a_message() {
        let collab = crate::collab::Collab::default();
        // The number 3 in MessagePack: what a step-1 build sends.
        let older = hardwave_protocol_song_state(vec![3u8]);
        assert!(
            collab.check_peer(&older).is_err(),
            "format 3 is another version"
        );
        let same = hardwave_protocol_song_state(Project::format_section());
        assert_eq!(collab.check_peer(&same), Ok(true));
    }

    fn hardwave_protocol_song_state(blob: Vec<u8>) -> hardwave_project::multiplayer::SyncKind {
        hardwave_project::multiplayer::SyncKind::SongState {
            section: Project::FORMAT_SECTION.to_string(),
            version: 0,
            author: 1,
            blob,
        }
    }
}
