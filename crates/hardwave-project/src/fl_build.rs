//! A Hardwave project made from a parsed FL Studio project.
//!
//! The import used to add to whatever song was open, put every track it
//! made after the five hundred empty insert rows (where the playlist
//! never showed them), and keep only the patterns, while a hardstyle
//! project is mostly samples on the playlist. Now it makes a song of its
//! own: the instruments on the first rows, then the playlist's audio
//! tracks in FL's order, every sample clip where FL had it, at the level,
//! pan, pitch, direction and length its channel gave it in FL.
//!
//! Some of that waits for the samples to be loaded, which happens outside
//! this crate: where an audio clip starts in its file (FL counts in
//! milliseconds, a clip in samples), how far a sample is stretched to fit
//! FL's TIME knob, and where a reversed clip reads from all need the
//! file's length. They are handed back as fix-ups to finish there.

use crate::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve, MidiClipRef};
use crate::fl_import::{FlChannel, FlChannelKind, FlClipContent, FlProject};
use crate::project::Project;
use std::collections::{BTreeMap, BTreeSet};

/// The song, and what is still to be done to it once the samples load.
pub struct FlBuild {
    pub project: Project,
    /// One per audio clip: what to finish once its file is loaded.
    pub audio_fixups: Vec<AudioFixup>,
    /// Sampler channels and the sample each one plays.
    pub sampler_samples: Vec<SamplerSample>,
    pub report: FlBuildReport,
}

/// A sampler channel's sample, with what FL did to it.
#[derive(Debug, Clone, PartialEq)]
pub struct SamplerSample {
    pub track_id: String,
    pub file: String,
    /// The key that plays the sample at its own pitch, with the channel's
    /// pitch knob (in whole semitones) taken into it.
    pub root_note: u8,
    pub reversed: bool,
    pub swap_stereo: bool,
    pub polarity_inverted: bool,
}

/// What an audio clip still needs once its file is loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioFixup {
    pub track_id: String,
    pub clip_id: String,
    /// Where the clip starts in its sample as FL plays it (after any stretch
    /// and reverse), in milliseconds; FL's -1 means "from the start".
    pub start_ms: f32,
    /// The clip's length on the timeline, in seconds.
    pub length_seconds: f64,
    /// FL's TIME knob in seconds at the song's tempo, when it is set: the
    /// whole sample is stretched to last this long.
    pub fit_seconds: Option<f64>,
    /// The MUL knob, on the length.
    pub multiplier: f64,
    /// Pitch in semitones from the channel's pitch, the stretch pitch and
    /// the root note.
    pub pitch_semitones: f64,
    /// FL's Resample mode: speed and pitch move together, like a tape.
    pub resample: bool,
    pub reversed: bool,
}

impl AudioFixup {
    /// Finish `clip` now that its file is known: `frames` long at `rate`.
    pub fn apply(&self, clip: &mut AudioClip, frames: u64, rate: u32) {
        if frames == 0 || rate == 0 {
            return;
        }
        let file_seconds = frames as f64 / rate as f64;
        // How much longer than the file the sample plays, before pitch.
        let length = match self.fit_seconds {
            Some(fit) if fit > 0.0 => fit / file_seconds,
            _ => 1.0,
        } * self.multiplier;
        let (stretch, pitch) = if self.resample {
            // A tape: raising the pitch plays it faster, and playing it
            // longer lowers it.
            (
                length * 2f64.powf(-self.pitch_semitones / 12.0),
                self.pitch_semitones - 12.0 * length.log2(),
            )
        } else {
            (length, self.pitch_semitones)
        };
        clip.stretch_ratio = stretch.clamp(0.1, 10.0);
        clip.pitch_semitones = pitch.clamp(-48.0, 48.0);
        let stretch = clip.stretch_ratio;

        // In the sample as played (stretched), in its frames.
        let played_frames = frames as f64 * stretch;
        let start = if self.start_ms > 0.0 {
            self.start_ms as f64 / 1000.0 * rate as f64
        } else {
            0.0
        };
        // The engine reads a reversed clip backwards from the end of its
        // window, where FL reverses the whole sample and starts the clip
        // `start` into that: the same sound when the window ends `start`
        // before the end of the file.
        let from = if self.reversed {
            (played_frames - start - self.length_seconds * rate as f64).max(0.0)
        } else {
            start
        };
        // Offsets count in the file's own frames; the engine scales them.
        let to = from + self.length_seconds * rate as f64;
        clip.source_start = ((from / stretch).round() as u64).min(frames);
        clip.source_end = ((to / stretch).round() as u64).clamp(clip.source_start, frames);
    }
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
    /// Channels using FL's quick fade knobs on their sample, which do not
    /// come across yet.
    pub sample_fades_left_behind: Vec<String>,
    /// Playlist tracks holding samples from channels panned, swapped or
    /// inverted differently, which a track here cannot be all at once.
    pub mixed_lanes: Vec<String>,
}

/// FL's channel volume knob (0 to 12800) in dB, 100 % being 0 dB.
///
/// FL shows its 78 % default as -5.2 dB and 75 % as -6.0 dB. A power curve
/// of 2.41 through 100 % = 0 dB gives both to the tenth.
pub fn fl_volume_db(raw: u32) -> f64 {
    if raw == 0 {
        return -100.0;
    }
    (48.2 * (raw as f64 / 12_800.0).log10()).max(-100.0)
}

/// An audio clip's gain from its channel's volume. FL plays an audio clip
/// at the file's own level when its channel sits at the 78 % default, so
/// the knob counts from there.
pub fn fl_audio_clip_gain_db(raw: u32) -> f64 {
    if raw == 0 {
        return -100.0;
    }
    (fl_volume_db(raw) - fl_volume_db(10_000)).max(-100.0)
}

/// FL's pan knob (0 to 12800, 6400 the centre) as -1 to 1.
pub fn fl_pan(raw: i32) -> f64 {
    ((raw as f64 - 6_400.0) / 6_400.0).clamp(-1.0, 1.0)
}

/// A sample channel's pitch in semitones: its pitch knob, the stretch
/// section's pitch and its root note (a higher root plays the sample
/// lower).
fn channel_semitones(channel: &FlChannel) -> f64 {
    let s = &channel.settings;
    (s.pitch_cents + s.stretch.pitch_cents) as f64 / 100.0 + (60.0 - s.root_note as f64)
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
    let seconds_per_tick = 60.0 / (fl.bpm.max(1.0) as f64 * hardwave_midi::PPQ as f64);
    let mut audio_fixups = Vec::new();
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
    let track_off = |fl_track: u32| fl.playlist_tracks_off.contains(&fl_track);
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
            let settings = &channel.settings;
            if let Some(track) = project.track_mut(&id) {
                track.volume_db = fl_volume_db(settings.volume);
                track.pan = fl_pan(settings.pan);
                track.muted = !settings.enabled;
            }
            match (channel.kind, &channel.sample_path) {
                (FlChannelKind::Sampler, Some(path)) => {
                    if let Some(file) = find(path) {
                        // The pitch knob moves the root the other way, so
                        // the notes stay the notes FL shows.
                        let root = settings.root_note as i32
                            - (settings.pitch_cents as f64 / 100.0).round() as i32;
                        sampler_samples.push(SamplerSample {
                            track_id: id.clone(),
                            file,
                            root_note: root.clamp(0, 127) as u8,
                            reversed: settings.reversed,
                            swap_stereo: settings.swap_stereo,
                            polarity_inverted: settings.polarity_inverted,
                        });
                    }
                    if settings.fade_in > 0 || settings.fade_out > 0 {
                        report.sample_fades_left_behind.push(name.clone());
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
        // A muted placement, or one on a track switched off, stays silent
        // here too, its notes kept.
        let silent = clip.muted || track_off(clip.track_index);
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
                    muted: silent,
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
    let mut faded = BTreeSet::new();
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

        // Pan, stereo swap and polarity belong to the channel in FL and to
        // the track here: they come across when the lane's channels agree.
        let settings: Vec<_> = channels
            .iter()
            .filter_map(|c| fl.channels.get(*c as usize))
            .map(|c| {
                (
                    c.settings.pan,
                    c.settings.swap_stereo,
                    c.settings.polarity_inverted,
                )
            })
            .collect();
        if let Some(first) = settings.first().copied() {
            if settings.iter().all(|s| *s == first) {
                if let Some(track) = project.track_mut(&track_id) {
                    track.pan = fl_pan(first.0);
                    track.swap_lr = first.1;
                    track.phase_invert = first.2;
                }
            } else {
                report.mixed_lanes.push(lane_name.clone());
            }
        }
        if track_off(fl_track) {
            if let Some(track) = project.track_mut(&track_id) {
                track.muted = true;
            }
        }

        for clip in clips {
            let FlClipContent::AudioSample {
                channel_index,
                sample_path,
                start_offset_ms,
                ..
            } = &clip.content
            else {
                continue;
            };
            let channel = fl.channels.get(*channel_index as usize);
            let settings = channel.map(|c| c.settings.clone()).unwrap_or_default();
            if (settings.fade_in > 0 || settings.fade_out > 0) && faded.insert(*channel_index) {
                report
                    .sample_fades_left_behind
                    .push(channel_name(*channel_index));
            }
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
                gain_db: fl_audio_clip_gain_db(settings.volume),
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: clip.muted || !settings.enabled,
                reversed: settings.reversed,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                fade_in_curve: FadeCurve::default(),
                fade_out_curve: FadeCurve::default(),
                warp_markers: Vec::new(),
                source_file: file,
            };
            let stretch = settings.stretch;
            audio_fixups.push(AudioFixup {
                track_id: track_id.clone(),
                clip_id,
                start_ms: *start_offset_ms,
                length_seconds: clip.length_ticks as f64 * seconds_per_tick,
                fit_seconds: (stretch.time_ticks > 0.0)
                    .then_some(stretch.time_ticks * seconds_per_tick),
                multiplier: stretch.multiplier,
                pitch_semitones: channel.map(channel_semitones).unwrap_or(0.0),
                resample: stretch.is_resample(),
                reversed: settings.reversed,
            });
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
        audio_fixups,
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
            settings: Default::default(),
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
                    muted: false,
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
                    muted: false,
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
                    muted: false,
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
                    muted: false,
                },
                FlPlaylistClip {
                    track_index: 2,
                    start_tick: 0,
                    length_ticks: 960,
                    content: FlClipContent::Automation {
                        channel_index: 3,
                        target: "Cutoff".into(),
                    },
                    muted: false,
                },
            ],
            mixer: Vec::new(),
            playlist_track_names: vec![(9, "Vocals".into())],
            playlist_tracks_off: Vec::new(),
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
        assert_eq!(built.audio_fixups.len(), 3, "one for every audio clip");
        assert_eq!(built.audio_fixups[0].start_ms, 20.0);
        assert_eq!(
            built.sampler_samples,
            vec![SamplerSample {
                track_id: "insert-002".to_string(),
                file: "/home/me/packs\\808.wav".to_string(),
                root_note: 60,
                reversed: false,
                swap_stereo: false,
                polarity_inverted: false,
            }]
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

    #[test]
    fn fl_volume_reads_as_fl_shows_it() {
        // FL's hints: 78 % is -5.2 dB, 75 % is -6.0 dB, 100 % is 0 dB.
        assert_eq!(format!("{:.1}", fl_volume_db(10_000)), "-5.2");
        assert_eq!(format!("{:.1}", fl_volume_db(9_600)), "-6.0");
        assert_eq!(fl_volume_db(12_800), 0.0);
        assert_eq!(fl_volume_db(0), -100.0);
        // An audio clip at the default plays the file as it is.
        assert!(fl_audio_clip_gain_db(10_000).abs() < 1e-9);
        assert!(fl_audio_clip_gain_db(7_456) < -6.0 && fl_audio_clip_gain_db(7_456) > -6.3);
        assert_eq!(fl_pan(6_400), 0.0);
        assert_eq!(fl_pan(0), -1.0);
        assert_eq!(fl_pan(12_800), 1.0);
    }

    fn lane_song() -> FlProject {
        let mut fl = song();
        // The kick render: quieter, reversed, panned right.
        let kick = &mut fl.channels[1].settings;
        kick.volume = 7_456;
        kick.reversed = true;
        kick.pan = 9_600;
        // The lost vocal's channel is switched off in the rack.
        fl.channels[4].settings.enabled = false;
        // The sampler: pitched down two semitones, louder than default.
        fl.channels[2].settings.pitch_cents = -200;
        fl.channels[2].settings.volume = 12_800;
        // The second kick placement is muted on the playlist, and the
        // vocals' track is switched off.
        fl.playlist_clips[2].muted = true;
        fl.playlist_tracks_off = vec![9];
        fl
    }

    #[test]
    fn a_channel_s_settings_reach_its_clips_and_tracks() {
        let built = build_project(&lane_song(), "Industrial", &here);
        let p = &built.project;
        let kicks = p.track("insert-003").unwrap();
        assert_eq!(kicks.pan, 0.5, "the lane's one channel pans the track");
        let audio = |i: usize| match &kicks.clips[i].content {
            ClipContent::Audio(a) => a.clone(),
            _ => panic!("expected audio"),
        };
        assert!((audio(0).gain_db - fl_audio_clip_gain_db(7_456)).abs() < 1e-9);
        assert!(audio(0).reversed);
        assert!(!audio(0).muted);
        assert!(audio(1).muted, "muted on the playlist");
        let vocals = p.track("insert-004").unwrap();
        assert!(vocals.muted, "its FL track is off");
        match &vocals.clips[0].content {
            ClipContent::Audio(a) => assert!(a.muted, "its channel is off"),
            _ => panic!("expected audio"),
        }
        let sampler = p.track("insert-002").unwrap();
        assert_eq!(sampler.volume_db, 0.0);
        assert_eq!(
            built.sampler_samples[0].root_note, 62,
            "two semitones down is the root two up"
        );
        let screech = p.track("insert-001").unwrap();
        assert_eq!(format!("{:.1}", screech.volume_db), "-5.2");
    }

    #[test]
    fn lanes_whose_channels_disagree_are_named() {
        let mut fl = lane_song();
        fl.channels[4].settings.pan = 0;
        fl.playlist_clips[3].track_index = 5;
        let built = build_project(&fl, "Industrial", &here);
        assert_eq!(built.report.mixed_lanes, vec!["Track 5".to_string()]);
        assert_eq!(built.project.track("insert-003").unwrap().pan, 0.0);
    }

    #[test]
    fn placements_on_a_switched_off_track_keep_their_notes_silent() {
        let mut fl = song();
        fl.playlist_tracks_off = vec![1];
        let built = build_project(&fl, "Industrial", &here);
        match &built.project.track("insert-001").unwrap().clips[0].content {
            ClipContent::Midi(m) => assert!(m.clip.notes.iter().all(|n| n.muted)),
            _ => panic!("expected notes"),
        }
    }

    fn fixup() -> AudioFixup {
        AudioFixup {
            track_id: "t".into(),
            clip_id: "c".into(),
            start_ms: -1.0,
            length_seconds: 1.0,
            fit_seconds: None,
            multiplier: 1.0,
            pitch_semitones: 0.0,
            resample: false,
            reversed: false,
        }
    }

    fn blank_clip() -> AudioClip {
        AudioClip {
            id: "c".into(),
            name: String::new(),
            source_path: String::new(),
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
            source_file: String::new(),
        }
    }

    #[test]
    fn the_time_knob_stretches_the_sample_to_fit() {
        // A two second file set to last four seconds.
        let mut clip = blank_clip();
        AudioFixup {
            fit_seconds: Some(4.0),
            resample: false,
            pitch_semitones: 1.0,
            ..fixup()
        }
        .apply(&mut clip, 96_000, 48_000);
        assert!((clip.stretch_ratio - 2.0).abs() < 1e-9);
        assert_eq!(clip.pitch_semitones, 1.0, "a stretch mode keeps the pitch");

        // The same in Resample: a tape at half speed, an octave down.
        let mut tape = blank_clip();
        AudioFixup {
            fit_seconds: Some(4.0),
            resample: true,
            ..fixup()
        }
        .apply(&mut tape, 96_000, 48_000);
        assert!((tape.stretch_ratio - 2.0).abs() < 1e-9);
        assert!((tape.pitch_semitones + 12.0).abs() < 1e-9);

        // Resample with only the pitch knob: up an octave plays twice as fast.
        let mut up = blank_clip();
        AudioFixup {
            resample: true,
            pitch_semitones: 12.0,
            ..fixup()
        }
        .apply(&mut up, 96_000, 48_000);
        assert!((up.stretch_ratio - 0.5).abs() < 1e-9);
        assert!((up.pitch_semitones - 12.0).abs() < 1e-9);
    }

    #[test]
    fn offsets_count_in_the_file_however_it_is_stretched() {
        // FL's offset is in the sample as played: one second into a sample
        // stretched to twice its length is half a second into the file.
        let mut clip = blank_clip();
        AudioFixup {
            start_ms: 1_000.0,
            fit_seconds: Some(4.0),
            ..fixup()
        }
        .apply(&mut clip, 96_000, 48_000);
        assert_eq!(clip.source_start, 24_000);
        assert_eq!(clip.source_end, 48_000, "one second of timeline later");
    }

    #[test]
    fn a_reversed_clip_reads_the_end_of_its_file() {
        // A two second file, reversed, a one second clip from its start (FL's
        // start of the reversed sample is the end of the file).
        let mut clip = blank_clip();
        AudioFixup {
            reversed: true,
            ..fixup()
        }
        .apply(&mut clip, 96_000, 48_000);
        assert_eq!(clip.source_start, 48_000);
        // Starting a quarter second into the reversed sample ends the window
        // a quarter second before the end of the file.
        let mut later = blank_clip();
        AudioFixup {
            reversed: true,
            start_ms: 250.0,
            ..fixup()
        }
        .apply(&mut later, 96_000, 48_000);
        assert_eq!(later.source_start, 36_000);
        assert_eq!(later.source_end, 84_000);
    }
}
