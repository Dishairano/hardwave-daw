//! A song saved before FL's three lists (format 2) opens laid out the way
//! it looked, and plays sample for sample as it did: opening it only fills
//! in where things show, never anything that makes sound.
//!
//! Each song here is saved as an older build saved it, then rendered twice
//! with the offline render: as that build held it, and as this one opens
//! it. The two renders must be the same, bit for bit.
//!
//! `HW_SONG_CORPUS` names a folder of `.hwp` files (songs from the test
//! machine) to check the same way; `HW_FL_SONG` an FL Studio project with
//! its samples beside it. Both are skipped when unset, and neither file is
//! ever part of the repository.

mod common;

use common::*;
use hardwave_engine::{AudioBuffer, DawEngine};
use hardwave_project::automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveMode};
use hardwave_project::automation_clip::AutomationClip;
use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};
use hardwave_project::track::{NativeInstrument, Track};
use hardwave_project::{Arrangement, Project, TrackTimeline};

const RATE: u32 = 48_000;

/// Where a render's samples come from: made here, or read from disk the
/// way a song's samples are when it opens.
enum Samples<'a> {
    Made(&'a [(String, AudioBuffer)]),
    FromDisk,
}

/// Render the start of a song.
fn render(project: Project, samples: &Samples, seconds: f64) -> Vec<f32> {
    let engine = DawEngine::new();
    match samples {
        Samples::Made(made) => {
            for (id, buffer) in made.iter() {
                engine.audio_pool.insert(id.clone(), buffer.clone());
            }
        }
        Samples::FromDisk => {
            engine
                .audio_source_loader(None)
                .load_sources_of(&project, |_, _| {});
        }
    }
    *engine.project.lock() = project;
    let mut out = Vec::new();
    engine
        .render_offline(RATE, (RATE as f64 * seconds) as u64, |block| {
            out.extend_from_slice(block);
            true
        })
        .unwrap();
    out
}

/// Both renders, sample for sample.
fn assert_same(name: &str, before: &[f32], after: &[f32]) {
    assert_eq!(
        before.len(),
        after.len(),
        "{name}: renders of another length"
    );
    if let Some(i) = before
        .iter()
        .zip(after)
        .position(|(a, b)| a.to_bits() != b.to_bits())
    {
        panic!(
            "{name}: sample {i} was {} and is now {}",
            before[i], after[i]
        );
    }
}

/// Save `song` as format 2, then check what this build opens: laid out,
/// and sounding as the older build had it.
fn opens_the_same(name: &str, song: &Project, samples: &Samples, seconds: f64) {
    let bytes = song.older_format_bytes();
    // What the older build held: the file read as it is.
    let mut before = Project::read_as_saved(&bytes).unwrap();
    assert!(before.rows.is_empty() && before.tracks.iter().all(|t| t.mixer_number.is_none()));
    before.make_safe().unwrap();
    let after = Project::from_bytes(&bytes).unwrap();
    assert_eq!(after.version, hardwave_project::project::FORMAT_VERSION);
    assert!(
        after.tracks.len() <= 1 || !after.rows.is_empty(),
        "{name} is laid out on opening"
    );
    let heard = render(before, samples, seconds);
    assert!(
        heard.iter().any(|s| s.abs() > 1e-4),
        "{name} makes a sound to compare"
    );
    assert_same(name, &heard, &render(after, samples, seconds));
}

fn tone() -> AudioBuffer {
    make_sine_buffer(RATE, 2.0, 220.0, 0.4)
}

fn audio(track: &str, source: &str, frames: u64, at: u64, length: u64) -> ClipPlacement {
    ClipPlacement {
        content: ClipContent::Audio(make_audio_clip(source, frames)),
        track_id: track.into(),
        position_ticks: at,
        length_ticks: length,
        lane: 0,
        color: None,
        row: None,
    }
}

fn point(tick: u64, value: f64) -> AutomationPoint {
    AutomationPoint {
        tick,
        value,
        curve: CurveMode::Linear,
        tension: 0.0,
    }
}

/// A song made in the DAW: an instrument with notes and an automation
/// clip, a sample track into a bus with a volume lane, a track after the
/// 500 inserts, a muted one, and a second arrangement.
fn daw_song() -> (Project, Vec<(String, AudioBuffer)>) {
    let tone = tone();
    let frames = tone.num_frames as u64;
    let mut p = Project::default();
    let lead = p.claim_row(1, "Lead", true);
    {
        let t = p.track_mut(&lead).unwrap();
        t.instrument = NativeInstrument::BuiltinSaw;
        t.volume_db = -6.0;
        let mut clip = hardwave_midi::MidiClip::new("lead-clip".into(), "Lead".into(), 7680);
        clip.notes = (0..16)
            .map(|i| hardwave_midi::MidiNote {
                start_tick: i * 480,
                duration_ticks: 400,
                pitch: 57 + (i % 4) as u8 * 3,
                velocity: 0.8,
                ..Default::default()
            })
            .collect();
        t.clips.push(ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: "lead-clip".into(),
                clip,
            }),
            track_id: lead.clone(),
            position_ticks: 0,
            length_ticks: 7680,
            lane: 0,
            color: None,
            row: None,
        });
        let mut pan = AutomationClip::new("lead-pan", AutomationTarget::TrackPan, 0, 7680);
        pan.lane.points = vec![point(0, 0.1), point(7680, 0.9)];
        t.automation_clips.push(pan);
    }
    let drums = p.claim_row(2, "Drums", false);
    let mut bus = Track::new_bus("bus".into(), "Drum bus".into());
    bus.volume_db = -3.0;
    p.tracks.push(bus);
    {
        let t = p.track_mut(&drums).unwrap();
        t.output_bus = Some("bus".into());
        for at in [0u64, 1920, 3840, 5760] {
            t.clips.push(audio(&drums, "tone", frames, at, 1440));
        }
        t.automation_lanes.push(AutomationLane {
            id: "drums-vol".into(),
            target: AutomationTarget::TrackVolume,
            points: vec![point(0, 0.8), point(7680, 0.3)],
            visible: true,
            row: None,
        });
    }
    let late = p.add_audio_track("Late".into());
    p.track_mut(&late)
        .unwrap()
        .clips
        .push(audio(&late, "tone", frames, 960, 2880));
    let muted = p.add_audio_track("Muted".into());
    {
        let t = p.track_mut(&muted).unwrap();
        t.muted = true;
        t.clips.push(audio(&muted, "tone", frames, 0, 7680));
    }
    p.ensure_arrangements();
    let mut other = TrackTimeline::default();
    other.clips.push(audio(&drums, "tone", frames, 7680, 1920));
    p.arrangements.push(Arrangement {
        id: "other".into(),
        name: "Other".into(),
        timelines: [(drums.clone(), other)].into_iter().collect(),
        rows: Vec::new(),
    });
    (p, vec![("tone".into(), tone)])
}

#[test]
fn a_song_made_in_the_daw_opens_and_plays_as_it_did() {
    let (song, samples) = daw_song();
    opens_the_same("the DAW song", &song, &Samples::Made(&samples), 4.0);
}

#[test]
fn the_dawproject_example_opens_and_plays_as_it_did() {
    let xml = include_str!("../../hardwave-project/src/testdata/dawproject_bitwig_example.xml");
    let imported = hardwave_project::dawproject_import::import(xml, "Example").unwrap();
    let song = imported.project;
    // Its audio files are not here; its notes play the built-in synth.
    opens_the_same("the DAWproject example", &song, &Samples::Made(&[]), 4.0);
}

#[test]
fn an_fl_import_opens_and_plays_as_it_did() {
    use hardwave_project::fl_import::*;
    let channel = |name: &str, kind, sample: Option<&str>, insert| FlChannel {
        name: name.into(),
        color: None,
        layer_children: Vec::new(),
        sample_path: sample.map(str::to_string),
        plugin_name: None,
        pattern_steps: Vec::new(),
        kind,
        settings: FlChannelSettings {
            mixer_insert: insert,
            ..Default::default()
        },
        plugin_data: Vec::new(),
        automation: Vec::new(),
    };
    let sample = |channel_index| FlClipContent::AudioSample {
        channel_index,
        sample_path: "C:\\packs\\tone.wav".into(),
        start_offset_ms: -1.0,
        end_offset_ms: -1.0,
        fade_in_ms: 0.0,
        fade_out_ms: 0.0,
        volume: 1.0,
    };
    let placed = |track_index, start_tick, channel| FlPlaylistClip {
        track_index,
        start_tick,
        length_ticks: 1920,
        content: sample(channel),
        muted: false,
        arrangement: 0,
    };
    let to_master = FlRoute {
        to: 0,
        level: 12_800,
    };
    let fl = FlProject {
        bpm: 150.0,
        time_sig_numerator: 4,
        time_sig_denominator: 4,
        channels: vec![
            channel(
                "Tone",
                FlChannelKind::AudioClip,
                Some("C:\\packs\\tone.wav"),
                1,
            ),
            channel(
                "Tone 2",
                FlChannelKind::AudioClip,
                Some("C:\\packs\\tone.wav"),
                2,
            ),
        ],
        // Two channels on one row, each through its own insert.
        playlist_clips: vec![placed(1, 0, 0), placed(1, 1920, 1), placed(3, 960, 0)],
        mixer: vec![
            FlMixerTrack::default(),
            FlMixerTrack {
                index: 1,
                name: "Tone bus".into(),
                volume: 9_000,
                routes: vec![to_master],
                ..Default::default()
            },
            FlMixerTrack {
                index: 2,
                volume: 11_000,
                pan: -3_200,
                routes: vec![to_master],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let built =
        hardwave_project::fl_build::build_project(&fl, "FL song", &|p: &str| Some(p.to_string()));
    let tone = tone();
    let mut song = built.project;
    for fixup in &built.audio_fixups {
        for track in song.tracks.iter_mut().filter(|t| t.id == fixup.track_id) {
            for placement in &mut track.clips {
                if let ClipContent::Audio(clip) = &mut placement.content {
                    if clip.id == fixup.clip_id {
                        fixup.apply(clip, tone.num_frames as u64, RATE);
                    }
                }
            }
        }
    }
    let samples = [("C:\\packs\\tone.wav".to_string(), tone)];
    opens_the_same("an FL import", &song, &Samples::Made(&samples), 3.0);
}

/// Songs from the test machine: `HW_SONG_CORPUS=/path/to/folder`. Their
/// samples are found where the song names them, as when it is opened.
#[test]
fn songs_from_a_folder_open_and_play_as_they_did() {
    let Ok(dir) = std::env::var("HW_SONG_CORPUS") else {
        eprintln!("HW_SONG_CORPUS is not set; skipped");
        return;
    };
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("hwp") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let mut before = Project::read_as_saved(&bytes).unwrap();
        before.make_safe().unwrap();
        let after = Project::load(&path).unwrap();
        let name = path.display().to_string();
        let heard = render(before, &Samples::FromDisk, 20.0);
        assert_same(&name, &heard, &render(after, &Samples::FromDisk, 20.0));
        eprintln!("{name}: the same");
        checked += 1;
    }
    eprintln!("{checked} songs checked");
}

/// The founder's FL project: `HW_FL_SONG=/path/to/song.flp`, its samples
/// in the same folder.
#[test]
fn an_fl_project_from_a_folder_opens_and_plays_as_it_did() {
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
                    if clip.id != fixup.clip_id {
                        continue;
                    }
                    if let Some(buffer) = engine.audio_pool.get(&clip.source_path) {
                        fixup.apply(clip, buffer.num_frames as u64, buffer.sample_rate);
                    }
                }
            }
        }
    }
    drop(engine);
    opens_the_same("the FL project", &song, &Samples::FromDisk, 30.0);
}
