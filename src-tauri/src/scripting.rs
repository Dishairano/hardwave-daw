//! Running the user's own scripts.
//!
//! `scripting_api.rs` in hardwave-project has had a command list and a
//! macro recorder in it for months with nothing to run them: a command
//! enum is not scripting. This is the part that was missing.
//!
//! The language is Rhai, which is plain Rust with no C toolchain to
//! build, so a script runs the same on every machine we ship to. A
//! script does not touch the engine itself: it collects commands, and
//! they are applied afterwards, one by one, on the main thread. That
//! way a script that loops forever or throws half way cannot leave the
//! project in a state no undo can describe.

use hardwave_project::scripting_api::ScriptCommand;
use std::sync::{Arc, Mutex};

/// How long a script may run before it is stopped. A script is written
/// by the person using it, so this is a mistake guard rather than a
/// sandbox: a typo in a loop should not need the task manager.
const MAX_OPERATIONS: u64 = 2_000_000;
/// The most a script may ask for in one run, and print. A song edit
/// needs far fewer; past this a script is looping, not editing.
const MAX_COMMANDS: usize = 10_000;
const MAX_LINES: usize = 1_000;

/// What a script did.
#[derive(Debug, Default)]
pub struct ScriptRun {
    /// The commands it asked for, in order.
    pub commands: Vec<ScriptCommand>,
    /// Anything it printed.
    pub output: Vec<String>,
}

/// What a script can read of the song, made before it runs: `song.bpm`,
/// and `song.tracks`, each with its id, name, kind, volume (dB), pan,
/// mute, solo and arm, and its clips (id, name, kind, start and length in
/// ticks, muted), and for a MIDI clip its notes (tick, length, pitch,
/// velocity 0 to 127). A copy, so a script reads the song as it was when
/// it started, and what it asks for is applied after it ends.
///
/// Scripts could change a song but not look at it, so even "make every
/// note on the bass track quieter" could not be written.
pub fn song_view(project: &hardwave_project::Project) -> rhai::Map {
    use hardwave_project::clip::ClipContent;
    use rhai::{Array, Dynamic, Map};
    let mut song = Map::new();
    song.insert(
        "bpm".into(),
        Dynamic::from_float(
            project
                .tempo_map
                .entries
                .first()
                .map(|e| e.bpm)
                .unwrap_or(120.0),
        ),
    );
    let tracks: Array = project
        .tracks
        .iter()
        .map(|t| {
            let mut m = Map::new();
            m.insert("id".into(), t.id.clone().into());
            m.insert("name".into(), t.name.clone().into());
            m.insert("kind".into(), format!("{:?}", t.kind).into());
            m.insert("volume".into(), Dynamic::from_float(t.volume_db));
            m.insert("pan".into(), Dynamic::from_float(t.pan));
            m.insert("muted".into(), t.muted.into());
            m.insert("soloed".into(), t.soloed.into());
            m.insert("armed".into(), t.armed.into());
            let clips: Array = t
                .clips
                .iter()
                .map(|c| {
                    let mut cm = Map::new();
                    cm.insert("start".into(), (c.position_ticks as i64).into());
                    cm.insert("length".into(), (c.length_ticks as i64).into());
                    match &c.content {
                        ClipContent::Audio(a) => {
                            cm.insert("id".into(), a.id.clone().into());
                            cm.insert("name".into(), a.name.clone().into());
                            cm.insert("kind".into(), "audio".into());
                            cm.insert("muted".into(), a.muted.into());
                        }
                        ClipContent::Midi(m) => {
                            cm.insert("id".into(), m.id.clone().into());
                            cm.insert("name".into(), m.clip.name.clone().into());
                            cm.insert("kind".into(), "midi".into());
                            cm.insert("muted".into(), m.clip.muted.into());
                            let notes: Array = m
                                .clip
                                .notes
                                .iter()
                                .map(|n| {
                                    let mut nm = Map::new();
                                    nm.insert("tick".into(), (n.start_tick as i64).into());
                                    nm.insert("length".into(), (n.duration_ticks as i64).into());
                                    nm.insert("pitch".into(), (n.pitch as i64).into());
                                    nm.insert(
                                        "velocity".into(),
                                        ((n.velocity * 127.0).round().clamp(0.0, 127.0) as i64)
                                            .into(),
                                    );
                                    nm.into()
                                })
                                .collect();
                            cm.insert("notes".into(), notes.into());
                        }
                    }
                    cm.into()
                })
                .collect();
            m.insert("clips".into(), clips.into());
            m.into()
        })
        .collect();
    song.insert("tracks".into(), tracks.into());
    song
}

/// Run a script and collect what it asks for.
///
/// Nothing is applied here. The caller gets a list of commands and
/// decides when to run them, which is what makes a script one undo
/// step rather than fifty. Without a song to read: for tests.
#[cfg(test)]
pub fn run(source: &str) -> Result<ScriptRun, String> {
    run_with_song(source, rhai::Map::new())
}

/// `run`, with the song for the script to read as `song`.
pub fn run_with_song(source: &str, song: rhai::Map) -> Result<ScriptRun, String> {
    let collected: Arc<Mutex<Vec<ScriptCommand>>> = Arc::new(Mutex::new(Vec::new()));
    let printed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let mut engine = rhai::Engine::new();
    engine.set_max_operations(MAX_OPERATIONS);
    // A script has no business reading the disk or the network, and
    // Rhai's own modules are the only way in.
    engine.set_max_modules(0);
    // Nor taking the machine's memory: a string doubled forty times is
    // a terabyte, and Rhai sets no limit of its own. These are far past
    // anything a script that edits a song needs.
    engine.set_max_string_size(64 * 1024);
    engine.set_max_array_size(10_000);
    engine.set_max_map_size(10_000);
    engine.set_max_call_levels(64);
    engine.set_max_expr_depths(64, 32);

    {
        let printed = Arc::clone(&printed);
        engine.on_print(move |text| {
            if let Ok(mut lines) = printed.lock() {
                lines.push(text.to_string());
            }
        });
    }
    {
        let printed = Arc::clone(&printed);
        engine.on_debug(move |text, _source, position| {
            if let Ok(mut lines) = printed.lock() {
                if lines.len() < MAX_LINES {
                    lines.push(format!("{position}: {text}"));
                }
            }
        });
    }

    // Every function a script can call. Each one only records what was
    // asked for: nothing here touches the engine.
    macro_rules! record {
        ($name:literal, || $build:expr) => {{
            let collected = Arc::clone(&collected);
            engine.register_fn($name, move || {
                if let Ok(mut list) = collected.lock() {
                    if list.len() < MAX_COMMANDS {
                        list.push($build);
                    }
                }
            });
        }};
        ($name:literal, |$($arg:ident : $ty:ty),+| $build:expr) => {{
            let collected = Arc::clone(&collected);
            engine.register_fn($name, move |$($arg : $ty),+| {
                if let Ok(mut list) = collected.lock() {
                    if list.len() < MAX_COMMANDS {
                        list.push($build);
                    }
                }
            });
        }};
    }

    record!("play", || ScriptCommand::TransportPlay);
    record!("stop", || ScriptCommand::TransportStop);
    record!("seek", |tick: i64| ScriptCommand::TransportSeek {
        tick: tick.max(0) as u64
    });
    record!("set_volume", |track_id: String, db: f64| {
        ScriptCommand::SetTrackVolume {
            track_id,
            db: db as f32,
        }
    });
    record!("set_pan", |track_id: String, pan: f64| {
        ScriptCommand::SetTrackPan {
            track_id,
            pan: pan.clamp(-1.0, 1.0) as f32,
        }
    });
    record!("set_muted", |track_id: String, muted: bool| {
        ScriptCommand::SetTrackMuted { track_id, muted }
    });
    record!("set_master_volume", |db: f64| {
        ScriptCommand::SetMasterVolume { db: db as f32 }
    });
    record!("add_note", |clip_id: String,
                         tick: i64,
                         pitch: i64,
                         velocity: i64,
                         length_ticks: i64| {
        ScriptCommand::InsertNote {
            clip_id,
            tick: tick.max(0) as u64,
            pitch: pitch.clamp(0, 127) as u8,
            velocity: velocity.clamp(0, 127) as u8,
            length_ticks: length_ticks.max(1) as u64,
        }
    });
    record!("delete_note", |clip_id: String, tick: i64, pitch: i64| {
        ScriptCommand::DeleteNote {
            clip_id,
            tick: tick.max(0) as u64,
            pitch: pitch.clamp(0, 127) as u8,
        }
    });
    record!("move_clip", |clip_id: String, new_start_tick: i64| {
        ScriptCommand::MoveClip {
            clip_id,
            new_start_tick: new_start_tick.max(0) as u64,
        }
    });
    record!("delete_clip", |clip_id: String| ScriptCommand::DeleteClip {
        clip_id
    });
    record!("open_panel", |panel_id: String| ScriptCommand::OpenPanel {
        panel_id
    });

    // Ticks per beat, so a script can say where it means without
    // knowing the number.
    // Both a whole number and a fraction, because a loop counter is an
    // integer and half a beat is not.
    engine.register_fn("beats", |beats: f64| {
        (beats * hardwave_midi::PPQ as f64) as i64
    });
    engine.register_fn("beats", |beats: i64| beats * hardwave_midi::PPQ as i64);
    engine.register_fn("bars", |bars: f64| {
        (bars * 4.0 * hardwave_midi::PPQ as f64) as i64
    });
    engine.register_fn("bars", |bars: i64| bars * 4 * hardwave_midi::PPQ as i64);

    let mut scope = rhai::Scope::new();
    scope.push_constant("song", song);
    engine
        .run_with_scope(&mut scope, source)
        .map_err(|e| format!("{e}"))
        .map(|()| ScriptRun {
            commands: collected.lock().map(|l| l.clone()).unwrap_or_default(),
            output: printed.lock().map(|l| l.clone()).unwrap_or_default(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_cannot_take_the_machines_memory() {
        let started = std::time::Instant::now();
        let doubled = run("let s = \"x\"; loop { s += s; }");
        assert!(doubled.is_err(), "a string that doubles forever is stopped");
        let piled = run("let a = []; loop { a.push(1); }");
        assert!(piled.is_err());
        let chatty = run("for i in 0..100000 { play(); print(i); }").unwrap();
        assert!(chatty.commands.len() <= MAX_COMMANDS);
        // The limits stop a runaway script instead of leaving it running
        // forever. Sixty seconds is far above what they take, so a build
        // machine busy with other compiles does not fail this (it took over
        // twenty on HQ on 22 Oct 2026 while another workspace test ran).
        assert!(started.elapsed() < std::time::Duration::from_secs(60));
    }

    #[test]
    fn a_script_collects_the_commands_it_asks_for() {
        let run = run(r#"
            play();
            set_volume("track-1", -6.0);
            stop();
        "#)
        .expect("the script should run");
        assert_eq!(
            run.commands,
            vec![
                ScriptCommand::TransportPlay,
                ScriptCommand::SetTrackVolume {
                    track_id: "track-1".into(),
                    db: -6.0
                },
                ScriptCommand::TransportStop,
            ]
        );
    }

    #[test]
    fn a_loop_writes_a_line_of_notes() {
        let run = run(r#"
            for i in 0..4 {
                add_note("clip-1", beats(i), 36, 100, beats(0.25));
            }
        "#)
        .expect("the script should run");
        assert_eq!(run.commands.len(), 4);
        match &run.commands[2] {
            ScriptCommand::InsertNote { tick, pitch, .. } => {
                assert_eq!(*tick, 2 * hardwave_midi::PPQ);
                assert_eq!(*pitch, 36);
            }
            other => panic!("expected a note, got {other:?}"),
        }
    }

    #[test]
    fn printing_comes_back_as_output() {
        let run = run(r#"print("two tracks done");"#).expect("run");
        assert_eq!(run.output, vec!["two tracks done".to_string()]);
        assert!(run.commands.is_empty());
    }

    #[test]
    fn a_script_that_does_not_parse_says_why() {
        let error = run("play(").expect_err("this should not parse");
        assert!(!error.is_empty());
    }

    #[test]
    fn a_runaway_loop_is_stopped() {
        let error = run("loop { }").expect_err("a loop with no end should be stopped");
        assert!(
            error.to_lowercase().contains("operation"),
            "the message should say what happened: {error}"
        );
    }

    #[test]
    fn values_out_of_range_are_brought_back_in() {
        let run = run(r#"
            set_pan("t", 4.0);
            add_note("c", -10, 300, -5, 0);
        "#)
        .expect("run");
        assert_eq!(
            run.commands[0],
            ScriptCommand::SetTrackPan {
                track_id: "t".into(),
                pan: 1.0
            }
        );
        match &run.commands[1] {
            ScriptCommand::InsertNote {
                tick,
                pitch,
                velocity,
                length_ticks,
                ..
            } => {
                assert_eq!(*tick, 0);
                assert_eq!(*pitch, 127);
                assert_eq!(*velocity, 0);
                assert_eq!(*length_ticks, 1, "a note with no length is not a note");
            }
            other => panic!("expected a note, got {other:?}"),
        }
    }

    #[test]
    fn a_script_cannot_reach_the_disk() {
        // Rhai has no file or network functions of its own, and modules
        // are off, so an import is the only door and it is shut.
        let error = run(r#"import "std" as s;"#).expect_err("imports should be refused");
        assert!(!error.is_empty());
    }

    /// A script reads the song and acts on what it finds: here, every
    /// note louder than 100 on the bass is replaced by a quieter one.
    #[test]
    fn a_script_reads_the_song() {
        use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};
        let mut p = hardwave_project::Project::default();
        let id = p.add_midi_track("Bass".into());
        let mut clip = hardwave_midi::MidiClip::new("bass-1".into(), "Bass line".into(), 3840);
        for (tick, vel) in [(0u64, 0.9f32), (960, 0.5)] {
            clip.notes.push(hardwave_midi::MidiNote {
                start_tick: tick,
                duration_ticks: 480,
                pitch: 36,
                velocity: vel,
                ..Default::default()
            });
        }
        p.track_mut(&id).unwrap().clips.push(ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: "bass-1".into(),
                clip,
            }),
            track_id: id.clone(),
            position_ticks: 0,
            length_ticks: 3840,
            lane: 0,
            color: None,
        });
        let script = r#"
            for t in song.tracks {
                if t.name != "Bass" { continue; }
                for c in t.clips {
                    for n in c.notes {
                        if n.velocity > 100 {
                            delete_note(c.id, n.tick, n.pitch);
                            add_note(c.id, n.tick, n.pitch, 80, n.length);
                        }
                    }
                }
            }
            print(song.tracks.len());
        "#;
        let ran = run_with_song(script, song_view(&p)).unwrap();
        assert_eq!(
            ran.commands.len(),
            2,
            "the loud note, deleted and added back quieter"
        );
        assert_eq!(ran.output, vec![p.tracks.len().to_string()]);
    }
}
