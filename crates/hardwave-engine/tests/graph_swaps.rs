//! Playback builds a new audio graph after every edit, off the audio thread,
//! and swaps it in whenever it is ready. A song played through such swaps
//! must sound exactly like its export, sample for sample, wherever the
//! swaps land: voices, tails, filters, delays and automation carry on.
//!
//! `HW_FL_SONG` names an FL Studio project with its samples beside it and
//! `HW_SONG_CORPUS` a folder of `.hwp` songs, to check those the same way.
//! Both are skipped when unset; neither file is part of the repository.

mod common;

use common::*;
use hardwave_engine::DawEngine;
use hardwave_project::automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveMode};
use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};
use hardwave_project::track::NativeInstrument;
use hardwave_project::Project;

const RATE: u32 = 48_000;

/// The song rendered as an export, and played through graph swaps.
fn both_renders(engine: &DawEngine, seconds: f64, swap_every: usize) -> (Vec<f32>, Vec<f32>, u64) {
    let total = (RATE as f64 * seconds) as u64;
    let mut export = Vec::new();
    engine
        .render_offline(RATE, total, |block| {
            export.extend_from_slice(block);
            true
        })
        .unwrap();
    let mut played = Vec::new();
    let swaps = engine
        .prepare_offline_render(RATE, 0, None, |_| {})
        .run_with_graph_swaps(total, swap_every, |block| {
            played.extend_from_slice(block);
            true
        });
    (export, played, swaps)
}

fn assert_same(name: &str, export: &[f32], played: &[f32]) {
    assert_eq!(
        export.len(),
        played.len(),
        "{name}: renders of another length"
    );
    if let Some(i) = export
        .iter()
        .zip(played)
        .position(|(a, b)| a.to_bits() != b.to_bits())
    {
        panic!(
            "{name}: frame {} was {} in the export and {} through the swaps",
            i / 2,
            export[i],
            played[i]
        );
    }
}

fn part(track_id: &str, notes: &[(u64, u64, u8)]) -> ClipPlacement {
    let ppq = hardwave_midi::PPQ;
    let mut clip = hardwave_midi::MidiClip::new("part".into(), "part".into(), 16 * ppq);
    for &(start_tick, duration_ticks, pitch) in notes {
        clip.notes.push(hardwave_midi::MidiNote {
            start_tick,
            duration_ticks,
            pitch,
            velocity: 0.8,
            ..Default::default()
        });
    }
    ClipPlacement {
        content: ClipContent::Midi(MidiClipRef {
            id: format!("part-{track_id}"),
            clip,
        }),
        track_id: track_id.into(),
        position_ticks: 0,
        length_ticks: 16 * ppq,
        lane: 0,
        color: None,
        row: None,
    }
}

/// Long synth notes through a filter, a kick with its tail, a sample on a
/// delayed and filtered track with its level automated, a send to a bus.
#[test]
fn a_song_plays_through_graph_swaps_exactly_as_it_exports() {
    let ppq = hardwave_midi::PPQ;
    let engine = DawEngine::new();
    let sample = add_audio_track_with_sine(&engine, "Sample", "sine", RATE, 4.0, 330.0, 0.4);
    {
        let mut p = engine.project.lock();
        let lead = p.add_midi_track("Lead".into());
        let kick = p.add_midi_track("Kick".into());
        let bus = p.add_audio_track("Bus".into());
        let t = p.track_mut(&lead).unwrap();
        t.instrument = NativeInstrument::BuiltinSaw;
        t.filter_type = "lp".into();
        t.filter_cutoff_hz = 1_200.0;
        t.clips.push(part(
            &lead,
            &[(0, 6 * ppq, 45), (ppq, 5 * ppq, 52), (6 * ppq, 4 * ppq, 57)],
        ));
        t.output_bus = Some(bus.clone());
        let t = p.track_mut(&kick).unwrap();
        t.instrument = NativeInstrument::KickSynth;
        t.clips.push(part(
            &kick,
            &[
                (0, 100, 36),
                (ppq * 2, 100, 36),
                (ppq * 4, 100, 36),
                (ppq * 6, 100, 36),
            ],
        ));
        let t = p.track_mut(&sample).unwrap();
        t.filter_type = "hp".into();
        t.filter_cutoff_hz = 200.0;
        t.delay_samples = 900;
        t.automation_lanes.push(AutomationLane {
            id: "sample-vol".into(),
            target: AutomationTarget::TrackVolume,
            points: vec![
                AutomationPoint {
                    tick: 0,
                    value: 0.2,
                    curve: CurveMode::Linear,
                    tension: 0.0,
                },
                AutomationPoint {
                    tick: 8 * ppq,
                    value: 1.0,
                    curve: CurveMode::Linear,
                    tension: 0.0,
                },
            ],
            visible: true,
            row: None,
        });
    }
    let (export, played, swaps) = both_renders(&engine, 3.0, 4);
    assert!(
        export.iter().any(|s| s.abs() > 0.05),
        "the song makes a sound"
    );
    assert!(swaps > 2, "graphs were swapped in while it played: {swaps}");
    assert_same("made here", &export, &played);
}

/// The founder's FL project: `HW_FL_SONG=/path/to/song.flp`, its samples in
/// the same folder.
#[test]
#[ignore = "a song from outside the repository"]
fn an_fl_project_plays_through_graph_swaps_exactly_as_it_exports() {
    let Ok(file) = std::env::var("HW_FL_SONG") else {
        eprintln!("HW_FL_SONG is not set; skipped");
        return;
    };
    let file = std::path::PathBuf::from(file);
    let dir = file.parent().unwrap().to_path_buf();
    let fl = hardwave_project::flp_parser::parse(&std::fs::read(&file).unwrap()).unwrap();
    let resolve = |p: &str| {
        let name = p.rsplit(['\\', '/']).next()?;
        let f = dir.join(name);
        f.is_file().then(|| f.to_string_lossy().into_owned())
    };
    let built = hardwave_project::fl_build::build_project(&fl, "FL song", &resolve);
    let engine = DawEngine::new();
    engine
        .audio_source_loader(None)
        .load_sources_of(&built.project, |_, _| {});
    let mut song = built.project;
    for fixup in &built.audio_fixups {
        for track in song.tracks.iter_mut().filter(|t| t.id == fixup.track_id) {
            for placement in &mut track.clips {
                if let ClipContent::Audio(clip) = &mut placement.content {
                    if clip.id == fixup.clip_id {
                        if let Some(buffer) = engine.audio_pool.get(&clip.source_path) {
                            fixup.apply(clip, buffer.num_frames as u64, buffer.sample_rate);
                        }
                    }
                }
            }
        }
    }
    *engine.project.lock() = Project::from_bytes(&song.to_bytes().unwrap()).unwrap();
    let (export, played, swaps) = both_renders(&engine, 8.0, 3);
    eprintln!("{swaps} graphs swapped in");
    assert!(swaps > 0);
    assert_same("FL song", &export, &played);
}

/// Songs from the test machine: `HW_SONG_CORPUS=/path/to/folder`.
#[test]
#[ignore = "songs from outside the repository"]
fn songs_from_a_folder_play_through_graph_swaps_exactly_as_they_export() {
    let Ok(dir) = std::env::var("HW_SONG_CORPUS") else {
        eprintln!("HW_SONG_CORPUS is not set; skipped");
        return;
    };
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "hwp"))
        .collect();
    files.sort();
    for file in files {
        let song = Project::from_bytes(&std::fs::read(&file).unwrap()).unwrap();
        let engine = DawEngine::new();
        engine
            .audio_source_loader(None)
            .load_sources_of(&song, |_, _| {});
        *engine.project.lock() = song;
        let (export, played, swaps) = both_renders(&engine, 8.0, 3);
        let name = file.display().to_string();
        eprintln!("{name}: {swaps} graphs swapped in");
        assert_same(&name, &export, &played);
    }
}
