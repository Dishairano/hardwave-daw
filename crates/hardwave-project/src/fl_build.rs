//! A Hardwave project made from a parsed FL Studio project.
//!
//! The import used to add to whatever song was open, put every track it
//! made after the five hundred empty insert rows (where the playlist
//! never showed them), and keep only the patterns, while a hardstyle
//! project is mostly samples on the playlist. Now it makes a song of its
//! own: the instruments on the first rows, then the playlist's audio
//! tracks in FL's order, every sample clip where FL had it.
//!
//! Two things wait for the samples to be loaded, which happens outside
//! this crate: where an audio clip starts and ends in its file (FL counts
//! in milliseconds, a clip in samples) and the sample a sampler channel
//! plays. Both are handed back to be finished there.

use crate::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve, MidiClipRef};
use crate::fl_import::{FlChannelKind, FlClipContent, FlProject};
use crate::project::Project;
use std::collections::{BTreeMap, BTreeSet};

/// The song, and what is still to be done to it once the samples load.
pub struct FlBuild {
    pub project: Project,
    /// Audio clips that start or end inside their file:
    /// (track id, clip id, start ms, end ms), FL's -1 meaning "not set".
    pub audio_offsets: Vec<(String, String, f32, f32)>,
    /// Sampler channels and the file each one plays: (track id, file).
    pub sampler_samples: Vec<(String, String)>,
    pub report: FlBuildReport,
}

/// What came across, and what did not.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FlBuildReport {
    pub bpm: f32,
    pub instrument_tracks: usize,
    pub audio_tracks: usize,
    pub audio_clips: usize,
    pub pattern_clips: usize,
    pub notes: usize,
    /// Samples the project points at that were not found here.
    pub missing_samples: Vec<String>,
    /// Instrument channels whose plug-in could not come across.
    pub plugins_left_behind: Vec<String>,
    pub automation_clips: usize,
}

/// Make the song. `resolve` turns a sample path as FL wrote it into a
/// file on this machine, or None when it cannot be found.
pub fn build_project(
    fl: &FlProject,
    name: &str,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> FlBuild {
    let mut project = Project::default();
    project.metadata.name = name.to_string();
    if let Some(entry) = project.tempo_map.entries.get_mut(0) {
        entry.bpm = fl.bpm as f64;
        entry.time_sig_num = fl.time_sig_numerator.max(1) as u32;
        entry.time_sig_den = fl.time_sig_denominator.max(1) as u32;
    }
    let mut report = FlBuildReport {
        bpm: fl.bpm,
        ..Default::default()
    };
    let mut audio_offsets = Vec::new();
    let mut sampler_samples = Vec::new();
    let mut missing = BTreeSet::new();
    let mut find = |path: &str| -> Option<String> {
        let found = resolve(path);
        if found.is_none() {
            missing.insert(path.to_string());
        }
        found
    };
    let channel_name = |index: u32| -> String {
        fl.channels
            .get(index as usize)
            .map(|c| c.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Channel {index}"))
    };
    let mut row = 0usize;

    // An instrument track per channel that plays notes, on the first rows.
    let playing: BTreeSet<u32> = fl
        .pattern_notes
        .iter()
        .filter(|(_, channel, notes)| {
            !notes.is_empty()
                && !matches!(
                    fl.channels.get(*channel as usize).map(|c| c.kind),
                    Some(FlChannelKind::AudioClip | FlChannelKind::Automation)
                )
        })
        .map(|(_, channel, _)| *channel)
        .collect();
    let mut instrument_track: BTreeMap<u32, String> = BTreeMap::new();
    for channel_index in playing {
        row += 1;
        let name = channel_name(channel_index);
        let id = project.claim_row(row, &name, true);
        report.instrument_tracks += 1;
        if let Some(channel) = fl.channels.get(channel_index as usize) {
            match (channel.kind, &channel.sample_path) {
                (FlChannelKind::Sampler, Some(path)) => {
                    if let Some(file) = find(path) {
                        sampler_samples.push((id.clone(), file));
                    }
                }
                (FlChannelKind::Instrument | FlChannelKind::Layer, _) => {
                    report.plugins_left_behind.push(match &channel.plugin_name {
                        Some(plugin) if plugin != "Fruity Wrapper" => format!("{name} ({plugin})"),
                        _ => name.clone(),
                    });
                    // Its plug-in could not come, so the track would play its
                    // notes on the built-in test tone: a dozen tracks of
                    // leads and screeches as bare sines, summed at full
                    // level, came out as a harsh distorted noise. The notes
                    // stay; the track is muted until a plug-in is put on it.
                    if let Some(track) = project.track_mut(&id) {
                        track.muted = true;
                    }
                }
                _ => {}
            }
        }
        instrument_track.insert(channel_index, id);
    }

    // Each placement of a pattern brings its notes, per channel, to that
    // channel's track.
    let pattern_name = |pattern: u32| -> String {
        fl.pattern_names
            .iter()
            .find(|(id, _)| *id == pattern)
            .map(|(_, name)| name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Pattern {pattern}"))
    };
    for clip in &fl.playlist_clips {
        let FlClipContent::Pattern { pattern_index } = &clip.content else {
            continue;
        };
        for (pattern, channel, notes) in &fl.pattern_notes {
            if pattern != pattern_index || notes.is_empty() {
                continue;
            }
            let Some(track_id) = instrument_track.get(channel) else {
                continue;
            };
            let mut midi = hardwave_midi::MidiClip::new(
                uuid::Uuid::new_v4().to_string(),
                pattern_name(*pattern),
                clip.length_ticks.max(1),
            );
            // Notes past the end of the placement are cut off in FL too.
            midi.notes = notes
                .iter()
                .filter(|n| n.tick < clip.length_ticks)
                .map(|n| hardwave_midi::MidiNote {
                    start_tick: n.tick,
                    duration_ticks: n.length_ticks,
                    pitch: n.pitch,
                    velocity: n.velocity as f32 / 127.0,
                    channel: 0,
                    muted: false,
                    ..Default::default()
                })
                .collect();
            report.notes += midi.notes.len();
            if let Some(track) = project.track_mut(track_id) {
                track.clips.push(ClipPlacement {
                    content: ClipContent::Midi(MidiClipRef {
                        id: uuid::Uuid::new_v4().to_string(),
                        clip: midi,
                    }),
                    track_id: track_id.clone(),
                    position_ticks: clip.start_tick,
                    length_ticks: clip.length_ticks.max(1),
                    lane: 0,
                });
                report.pattern_clips += 1;
            }
        }
    }

    // The playlist's tracks that hold samples, in FL's order, after the
    // instruments.
    let mut audio_lanes: BTreeMap<u32, Vec<&crate::fl_import::FlPlaylistClip>> = BTreeMap::new();
    for clip in &fl.playlist_clips {
        match &clip.content {
            FlClipContent::AudioSample { .. } => {
                audio_lanes.entry(clip.track_index).or_default().push(clip)
            }
            FlClipContent::Automation { .. } => report.automation_clips += 1,
            FlClipContent::Pattern { .. } => {}
        }
    }
    for (fl_track, clips) in audio_lanes {
        let channels: BTreeSet<u32> = clips
            .iter()
            .filter_map(|c| match &c.content {
                FlClipContent::AudioSample { channel_index, .. } => Some(*channel_index),
                _ => None,
            })
            .collect();
        let lane_name = fl
            .playlist_track_names
            .iter()
            .find(|(n, _)| *n == fl_track)
            .map(|(_, name)| name.clone())
            .or_else(|| {
                (channels.len() == 1).then(|| channel_name(*channels.iter().next().unwrap_or(&0)))
            })
            .unwrap_or_else(|| format!("Track {fl_track}"));
        row += 1;
        let track_id = project.claim_row(row, &lane_name, false);
        report.audio_tracks += 1;
        for clip in clips {
            let FlClipContent::AudioSample {
                channel_index,
                sample_path,
                start_offset_ms,
                end_offset_ms,
            } = &clip.content
            else {
                continue;
            };
            // A sample that is not here still gets its clip, pointing at
            // where FL had it, so the song keeps its shape and the file
            // can be found later.
            let file = find(sample_path).unwrap_or_else(|| sample_path.clone());
            let clip_id = uuid::Uuid::new_v4().to_string();
            let audio = AudioClip {
                id: clip_id.clone(),
                name: channel_name(*channel_index),
                source_path: file.clone(),
                source_hash: String::new(),
                source_start: 0,
                source_end: 0,
                gain_db: 0.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                fade_in_curve: FadeCurve::default(),
                fade_out_curve: FadeCurve::default(),
                warp_markers: Vec::new(),
                source_file: file,
            };
            if *start_offset_ms > 0.0 || *end_offset_ms > 0.0 {
                audio_offsets.push((track_id.clone(), clip_id, *start_offset_ms, *end_offset_ms));
            }
            if let Some(track) = project.track_mut(&track_id) {
                track.clips.push(ClipPlacement {
                    content: ClipContent::Audio(audio),
                    track_id: track_id.clone(),
                    position_ticks: clip.start_tick,
                    length_ticks: clip.length_ticks.max(1),
                    lane: 0,
                });
                report.audio_clips += 1;
            }
        }
    }

    report.missing_samples = missing.into_iter().collect();
    FlBuild {
        project,
        audio_offsets,
        sampler_samples,
        report,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fl_import::{FlChannel, FlNote, FlPlaylistClip};

    fn channel(
        name: &str,
        kind: FlChannelKind,
        sample: Option<&str>,
        plugin: Option<&str>,
    ) -> FlChannel {
        FlChannel {
            name: name.into(),
            sample_path: sample.map(String::from),
            plugin_name: plugin.map(String::from),
            pattern_steps: Vec::new(),
            kind,
        }
    }

    fn song() -> FlProject {
        let note = |tick| FlNote {
            tick,
            length_ticks: 240,
            pitch: 60,
            velocity: 127,
        };
        FlProject {
            bpm: 160.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            channels: vec![
                channel(
                    "Screech",
                    FlChannelKind::Instrument,
                    None,
                    Some("Fruity Wrapper"),
                ),
                channel(
                    "Kick render",
                    FlChannelKind::AudioClip,
                    Some("C:\\packs\\kick.wav"),
                    None,
                ),
                channel(
                    "808 Kick",
                    FlChannelKind::Sampler,
                    Some("C:\\packs\\808.wav"),
                    None,
                ),
                channel("Cutoff", FlChannelKind::Automation, None, None),
                channel(
                    "Lost vocal",
                    FlChannelKind::AudioClip,
                    Some("D:\\gone\\vox.wav"),
                    None,
                ),
            ],
            notes: Vec::new(),
            pattern_notes: vec![(1, 0, vec![note(0), note(960)]), (1, 2, vec![note(0)])],
            pattern_names: vec![(1, "Drop".into())],
            playlist_clips: vec![
                FlPlaylistClip {
                    track_index: 1,
                    start_tick: 0,
                    length_ticks: 3840,
                    content: FlClipContent::Pattern { pattern_index: 1 },
                },
                FlPlaylistClip {
                    track_index: 5,
                    start_tick: 960,
                    length_ticks: 960,
                    content: FlClipContent::AudioSample {
                        channel_index: 1,
                        sample_path: "C:\\packs\\kick.wav".into(),
                        start_offset_ms: 20.0,
                        end_offset_ms: 400.0,
                    },
                },
                FlPlaylistClip {
                    track_index: 5,
                    start_tick: 1920,
                    length_ticks: 960,
                    content: FlClipContent::AudioSample {
                        channel_index: 1,
                        sample_path: "C:\\packs\\kick.wav".into(),
                        start_offset_ms: -1.0,
                        end_offset_ms: -1.0,
                    },
                },
                FlPlaylistClip {
                    track_index: 9,
                    start_tick: 0,
                    length_ticks: 960,
                    content: FlClipContent::AudioSample {
                        channel_index: 4,
                        sample_path: "D:\\gone\\vox.wav".into(),
                        start_offset_ms: -1.0,
                        end_offset_ms: -1.0,
                    },
                },
                FlPlaylistClip {
                    track_index: 2,
                    start_tick: 0,
                    length_ticks: 960,
                    content: FlClipContent::Automation {
                        channel_index: 3,
                        target: "Cutoff".into(),
                    },
                },
            ],
            mixer: Vec::new(),
            playlist_track_names: vec![(9, "Vocals".into())],
        }
    }

    fn here(path: &str) -> Option<String> {
        path.starts_with("C:\\packs")
            .then(|| path.replace("C:\\packs", "/home/me/packs"))
    }

    #[test]
    fn instruments_take_the_first_rows_and_samples_the_rows_after() {
        let built = build_project(&song(), "Industrial", &here);
        let p = &built.project;
        let row = |n: usize| p.track(&format!("insert-{n:03}")).expect("row");
        assert_eq!(row(1).name, "Screech");
        assert_eq!(row(2).name, "808 Kick");
        assert_eq!(
            row(3).name,
            "Kick render",
            "one channel on the lane names it"
        );
        assert_eq!(row(4).name, "Vocals", "a named FL track keeps its name");
        assert_eq!(row(5).name, "Insert 5", "the rest stay empty inserts");
        assert_eq!(p.metadata.name, "Industrial");
        assert_eq!(p.tempo_map.entries[0].bpm, 160.0);
    }

    #[test]
    fn patterns_land_on_their_channels_and_samples_where_fl_had_them() {
        let built = build_project(&song(), "Industrial", &here);
        let p = &built.project;
        let screech = p.track("insert-001").unwrap();
        assert_eq!(screech.clips.len(), 1);
        match &screech.clips[0].content {
            ClipContent::Midi(m) => {
                assert_eq!(m.clip.notes.len(), 2);
                assert_eq!(m.clip.name, "Drop");
            }
            _ => panic!("expected notes"),
        }
        let kicks = p.track("insert-003").unwrap();
        assert_eq!(kicks.clips.len(), 2);
        assert_eq!(kicks.clips[1].position_ticks, 1920);
        match &kicks.clips[0].content {
            ClipContent::Audio(a) => {
                assert_eq!(a.source_file, "/home/me/packs\\kick.wav");
                assert_eq!(a.source_path, a.source_file);
            }
            _ => panic!("expected audio"),
        }
        assert_eq!(built.report.audio_clips, 3);
        assert_eq!(built.report.pattern_clips, 2);
        assert_eq!(built.report.notes, 3);
    }

    #[test]
    fn what_waits_for_the_samples_and_what_did_not_come_is_said() {
        let built = build_project(&song(), "Industrial", &here);
        assert!(
            built.project.track("insert-001").unwrap().muted,
            "the track whose plug-in did not come is muted"
        );
        assert!(
            !built.project.track("insert-002").unwrap().muted,
            "the sampler track plays"
        );
        assert_eq!(built.audio_offsets.len(), 1, "only the trimmed clip");
        assert_eq!(
            (built.audio_offsets[0].2, built.audio_offsets[0].3),
            (20.0, 400.0)
        );
        assert_eq!(
            built.sampler_samples,
            vec![(
                "insert-002".to_string(),
                "/home/me/packs\\808.wav".to_string()
            )]
        );
        assert_eq!(
            built.report.missing_samples,
            vec!["D:\\gone\\vox.wav".to_string()]
        );
        assert_eq!(
            built.report.plugins_left_behind,
            vec!["Screech".to_string()]
        );
        assert_eq!(built.report.automation_clips, 1);
        // The missing sample still has its clip, pointing where FL had it.
        let vocals = built.project.track("insert-004").unwrap();
        assert_eq!(vocals.clips.len(), 1);
    }
}
