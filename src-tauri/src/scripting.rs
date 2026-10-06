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

/// What a script did.
#[derive(Debug, Default)]
pub struct ScriptRun {
    /// The commands it asked for, in order.
    pub commands: Vec<ScriptCommand>,
    /// Anything it printed.
    pub output: Vec<String>,
}

/// Run a script and collect what it asks for.
///
/// Nothing is applied here. The caller gets a list of commands and
/// decides when to run them, which is what makes a script one undo
/// step rather than fifty.
pub fn run(source: &str) -> Result<ScriptRun, String> {
    let collected: Arc<Mutex<Vec<ScriptCommand>>> = Arc::new(Mutex::new(Vec::new()));
    let printed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let mut engine = rhai::Engine::new();
    engine.set_max_operations(MAX_OPERATIONS);
    // A script has no business reading the disk or the network, and
    // Rhai's own modules are the only way in.
    engine.set_max_modules(0);

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
                lines.push(format!("{position}: {text}"));
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
                    list.push($build);
                }
            });
        }};
        ($name:literal, |$($arg:ident : $ty:ty),+| $build:expr) => {{
            let collected = Arc::clone(&collected);
            engine.register_fn($name, move |$($arg : $ty),+| {
                if let Ok(mut list) = collected.lock() {
                    list.push($build);
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

    engine
        .run(source)
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
}
