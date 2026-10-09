//! Starter songs: something that plays the moment it opens.
//!
//! A new producer who opens a free DAW and finds an empty timeline
//! closes it again. A starter song is a skeleton in the genre they came
//! for: the right tempo, a kick from the built-in kick synth already
//! tuned for the style, an offbeat bass, the usual sections, and a
//! limiter on the master. Pressing play makes a sound in the first
//! second, and everything in it is meant to be replaced.

use crate::AppState;
use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};
use hardwave_project::track::NativeInstrument;
use serde::Serialize;
use tauri::State;

const PPQ: u64 = hardwave_midi::PPQ;
const BAR: u64 = 4 * PPQ;

/// One genre's starting point.
struct Genre {
    name: &'static str,
    bpm: f64,
    kick_preset: &'static str,
    /// The kick note, which the kick synth uses for its pitch.
    kick_note: u8,
    /// Root of the offbeat bass, an octave and a bit above the kick.
    bass_note: u8,
}

fn genre(id: &str) -> Option<Genre> {
    Some(match id {
        "hardstyle" => Genre {
            name: "Hardstyle",
            bpm: 150.0,
            kick_preset: "Hardstyle Default",
            kick_note: 41,
            bass_note: 41,
        },
        "rawstyle" => Genre {
            name: "Rawstyle",
            bpm: 150.0,
            kick_preset: "Rawphoric Long",
            kick_note: 40,
            bass_note: 40,
        },
        "frenchcore" => Genre {
            name: "Frenchcore",
            bpm: 195.0,
            kick_preset: "Frenchcore Punch",
            kick_note: 38,
            bass_note: 50,
        },
        "uptempo" => Genre {
            name: "Uptempo",
            bpm: 200.0,
            kick_preset: "Uptempo Tight",
            kick_note: 39,
            bass_note: 51,
        },
        _ => return None,
    })
}

/// A section of the song, so the window can put markers on the ruler.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub name: String,
    pub start_tick: u64,
    pub bars: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StarterSong {
    pub genre: String,
    pub bpm: f64,
    pub sections: Vec<Section>,
}

/// The shape of every starter: long enough to feel like a song, short
/// enough to read at a glance.
fn sections() -> Vec<Section> {
    let plan: [(&str, u64); 6] = [
        ("Intro", 16),
        ("Build", 8),
        ("Drop", 16),
        ("Break", 16),
        ("Drop 2", 16),
        ("Outro", 8),
    ];
    let mut start = 0u64;
    plan.iter()
        .map(|(name, bars)| {
            let section = Section {
                name: name.to_string(),
                start_tick: start,
                bars: *bars,
            };
            start += bars * BAR;
            section
        })
        .collect()
}

pub(super) fn note(start: u64, length: u64, pitch: u8, velocity: f32) -> hardwave_midi::MidiNote {
    hardwave_midi::MidiNote {
        start_tick: start,
        duration_ticks: length.max(1),
        pitch,
        velocity,
        channel: 0,
        muted: false,
        ..Default::default()
    }
}

/// A kick on every beat, with a roll into the drop at the end of the
/// build, which is what makes it sound like the genre and not a
/// metronome.
fn kick_notes(bars: u64, pitch: u8, roll_last_bar: bool) -> Vec<hardwave_midi::MidiNote> {
    let mut notes = Vec::new();
    for bar in 0..bars {
        let rolling = roll_last_bar && bar == bars - 1;
        let step = if rolling { PPQ / 4 } else { PPQ };
        let hits = if rolling { 16 } else { 4 };
        for hit in 0..hits {
            let at = bar * BAR + hit * step;
            let velocity = if rolling {
                0.6 + 0.025 * hit as f32
            } else {
                1.0
            };
            notes.push(note(at, step * 9 / 10, pitch, velocity));
        }
    }
    notes
}

/// The offbeat bass: one note between every kick.
fn offbeat_bass(bars: u64, pitch: u8) -> Vec<hardwave_midi::MidiNote> {
    (0..bars * 4)
        .map(|beat| note(beat * PPQ + PPQ / 2, PPQ / 2 - PPQ / 16, pitch, 0.85))
        .collect()
}

pub(super) fn midi_clip(
    track_id: &str,
    name: &str,
    at: u64,
    length: u64,
    notes: Vec<hardwave_midi::MidiNote>,
) -> ClipPlacement {
    let mut clip =
        hardwave_midi::MidiClip::new(uuid::Uuid::new_v4().to_string(), name.to_string(), length);
    clip.notes = notes;
    ClipPlacement {
        content: ClipContent::Midi(MidiClipRef {
            id: uuid::Uuid::new_v4().to_string(),
            clip,
        }),
        track_id: track_id.to_string(),
        position_ticks: at,
        length_ticks: length,
        lane: 0,
    }
}

/// Put a starter song's tracks and clips into a project.
///
/// Kept apart from the command so a test can build one into an engine
/// and listen to it, without a window.
fn build_into(project: &mut hardwave_project::Project, genre: &Genre, plan: &[Section]) {
    let at = |name: &str| plan.iter().find(|s| s.name == name).cloned();
    // The kick: the built-in kick synth with the genre's preset,
    // playing everywhere except the break.
    let kick = claim_row(project, 1, "Kick", true);
    let layers = hardwave_dsp::kick_synth::preset_layers(genre.kick_preset);
    if let Some(track) = project.track_mut(&kick) {
        track.instrument = NativeInstrument::KickSynth;
        track.kick_patch.layers = [
            Some(crate::commands::tracks::layer_to_patch(&layers[0])),
            Some(crate::commands::tracks::layer_to_patch(&layers[1])),
            Some(crate::commands::tracks::layer_to_patch(&layers[2])),
            Some(crate::commands::tracks::layer_to_patch(&layers[3])),
        ];
        for (name, roll) in [
            ("Intro", false),
            ("Build", true),
            ("Drop", false),
            ("Drop 2", false),
            ("Outro", false),
        ] {
            if let Some(s) = at(name) {
                track.clips.push(midi_clip(
                    &kick,
                    &format!("Kick {name}"),
                    s.start_tick,
                    s.bars * BAR,
                    kick_notes(s.bars, genre.kick_note, roll),
                ));
            }
        }
    }

    // The offbeat bass, in the drops only.
    let bass = claim_row(project, 2, "Bass", true);
    if let Some(track) = project.track_mut(&bass) {
        track.instrument = NativeInstrument::BuiltinSaw;
        track.volume_db = -8.0;
        for name in ["Drop", "Drop 2"] {
            if let Some(s) = at(name) {
                track.clips.push(midi_clip(
                    &bass,
                    &format!("Bass {name}"),
                    s.start_tick,
                    s.bars * BAR,
                    offbeat_bass(s.bars, genre.bass_note),
                ));
            }
        }
    }

    // Empty places for the parts every song needs, so the next
    // step is obvious.
    for (row, name) in [(3, "Lead"), (4, "Screech"), (5, "Atmos")] {
        claim_row(project, row, name, true);
    }
    for (row, name) in [(6, "Vocals"), (7, "FX")] {
        claim_row(project, row, name, false);
    }
}

/// Take over playlist row `row` for a starter part.
///
/// A starter part takes a playlist row: see `Project::claim_row`.
pub(crate) fn claim_row(
    project: &mut hardwave_project::Project,
    row: usize,
    name: &str,
    instrument: bool,
) -> String {
    project.claim_row(row, name, instrument)
}

/// Build a starter song into the open project.
///
/// Called on a fresh project: it adds tracks rather than clearing, so
/// the window starts a new project first.
#[tauri::command]
pub fn create_starter_song(
    state: State<AppState>,
    genre_id: String,
) -> Result<StarterSong, String> {
    let genre = genre(&genre_id).ok_or_else(|| format!("no starter for {genre_id}"))?;
    let plan = sections();

    state.engine.lock().snapshot_before_mutation();
    let master_id = {
        let engine = state.engine.lock();
        engine
            .transport
            .bpm
            .store(genre.bpm, std::sync::atomic::Ordering::Relaxed);
        engine.send_command(hardwave_engine::transport::TransportCommand::SetBpm(
            genre.bpm,
        ));
        let mut project = engine.project.lock();
        if let Some(entry) = project.tempo_map.entries.get_mut(0) {
            entry.bpm = genre.bpm;
        }

        build_into(&mut project, &genre, &plan);

        project
            .tracks
            .iter()
            .find(|t| matches!(t.kind, hardwave_project::track::TrackKind::Master))
            .map(|t| t.id.clone())
    };

    {
        let engine = state.engine.lock();
        engine.sync_track_meters();
        engine.rebuild_graph();
    }

    // A limiter on the master, so a starter at full kick volume does
    // not clip the moment it plays.
    if let Some(master) = master_id {
        let _ = crate::commands::plugins::add_plugin_without_undo_step(
            state,
            master,
            hardwave_native_plugins::NativeLimiter::ID.to_string(),
        );
    }

    Ok(StarterSong {
        genre: genre.name.to_string(),
        bpm: genre.bpm,
        sections: plan,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sections_follow_each_other_without_gaps() {
        let plan = sections();
        let mut expected = 0;
        for s in &plan {
            assert_eq!(
                s.start_tick, expected,
                "{} starts where the last ended",
                s.name
            );
            expected += s.bars * BAR;
        }
        assert_eq!(plan.first().unwrap().name, "Intro");
        assert!(plan.iter().any(|s| s.name == "Drop"));
    }

    #[test]
    fn a_kick_lands_on_every_beat() {
        let notes = kick_notes(2, 41, false);
        assert_eq!(notes.len(), 8);
        assert!(notes.iter().all(|n| n.start_tick % PPQ == 0));
    }

    #[test]
    fn the_build_rolls_into_the_drop() {
        let notes = kick_notes(8, 41, true);
        // Seven bars of four, then a bar of sixteenths.
        assert_eq!(notes.len(), 7 * 4 + 16);
        let last_bar: Vec<_> = notes.iter().filter(|n| n.start_tick >= 7 * BAR).collect();
        assert_eq!(last_bar.len(), 16);
        assert!(
            last_bar.last().unwrap().velocity > last_bar.first().unwrap().velocity,
            "a roll gets louder towards the drop"
        );
    }

    #[test]
    fn the_bass_sits_between_the_kicks() {
        let notes = offbeat_bass(1, 41);
        assert_eq!(notes.len(), 4);
        assert!(
            notes.iter().all(|n| n.start_tick % PPQ == PPQ / 2),
            "every note on the offbeat"
        );
        assert!(
            notes
                .iter()
                .all(|n| n.start_tick + n.duration_ticks < (n.start_tick / PPQ + 1) * PPQ),
            "and gone before the next kick"
        );
    }

    #[test]
    fn a_starter_song_makes_a_sound_the_moment_it_plays() {
        let engine = hardwave_engine::DawEngine::new();
        let genre = genre("hardstyle").unwrap();
        {
            let mut project = engine.project.lock();
            build_into(&mut project, &genre, &sections());
        }
        engine.rebuild_graph();

        // The first two seconds of the intro: the kick is there.
        let mut peak = 0.0f32;
        let mut broken = 0usize;
        engine
            .render_offline(48_000, 96_000, |block| {
                for s in block {
                    if !s.is_finite() {
                        broken += 1;
                    } else {
                        peak = peak.max(s.abs());
                    }
                }
                true
            })
            .expect("the starter renders");
        assert_eq!(broken, 0, "no NaN or infinity in a starter");
        assert!(
            peak > 0.05,
            "pressing play has to make a sound: peak {peak}"
        );
    }

    #[test]
    fn the_starter_fills_the_first_playlist_rows() {
        let mut project = hardwave_project::Project::default();
        build_into(&mut project, &genre("hardstyle").unwrap(), &sections());
        let names: Vec<(&str, &str)> = project
            .tracks
            .iter()
            .filter(|t| t.id.starts_with("insert-"))
            .take(7)
            .map(|t| (t.id.as_str(), t.name.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("insert-001", "Kick"),
                ("insert-002", "Bass"),
                ("insert-003", "Lead"),
                ("insert-004", "Screech"),
                ("insert-005", "Atmos"),
                ("insert-006", "Vocals"),
                ("insert-007", "FX"),
            ]
        );
        assert!(!project.track("insert-001").unwrap().clips.is_empty());
        assert_eq!(
            project.tracks.len(),
            hardwave_project::project::DEFAULT_INSERT_COUNT + 1,
            "no rows added past the end, where nobody sees them"
        );
    }

    #[test]
    fn every_genre_has_a_tempo_and_a_kick_preset_that_exists() {
        for id in ["hardstyle", "rawstyle", "frenchcore", "uptempo"] {
            let g = genre(id).expect(id);
            assert!(g.bpm >= 140.0 && g.bpm <= 220.0);
            assert!(
                hardwave_dsp::kick_synth::PRESET_NAMES.contains(&g.kick_preset),
                "{} uses a preset the kick synth knows",
                g.name
            );
        }
        assert!(genre("polka").is_none());
    }
}
