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

use crate::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve, MidiClipRef, SampleTreatment};
use crate::fl_import::{
    read_wrapper, FlChannel, FlChannelKind, FlClipContent, FlMixerTrack, FlPluginFormat, FlProject,
    FlRoute, FlWrappedPlugin, WRAPPER,
};
use crate::project::Project;
use crate::track::TrackKind;
use std::collections::{BTreeMap, BTreeSet};

/// The song, and what is still to be done to it once the samples load.
pub struct FlBuild {
    pub project: Project,
    /// One per audio clip: what to finish once its file is loaded.
    pub audio_fixups: Vec<AudioFixup>,
    /// Sampler channels and the sample each one plays.
    pub sampler_samples: Vec<SamplerSample>,
    /// Built-in effects standing in for FL's, to be given their settings.
    pub native_effects: Vec<NativeEffect>,
    /// Other people's plug-ins, to be found on this computer.
    pub external_plugins: Vec<ExternalPlugin>,
    pub report: FlBuildReport,
}

/// Someone else's plug-in from FL: its slot is on its track, set as FL had
/// it. Whether it is on this computer is found out where the plug-ins are
/// known; until then the slot names it by `candidates[0]`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalPlugin {
    pub track_id: String,
    pub slot_id: String,
    /// Where it was in FL, for the report: a channel, or "insert: plug-in".
    pub place: String,
    pub plugin: FlWrappedPlugin,
    /// The ids it may have here, most likely first.
    pub candidates: Vec<String>,
    /// It plays its track's notes: without it the track stays muted.
    pub plays_track: bool,
}

/// The ids a plug-in from FL may have in our plug-in list: by its name, and
/// by its file's name, the way the scanner names them. A VST2 is looked for
/// as its VST3.
fn plugin_ids(plugin: &FlWrappedPlugin) -> Vec<String> {
    let id = |s: &str| s.to_lowercase().replace(' ', "-");
    let stem = plugin
        .file
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .split('.')
        .next()
        .unwrap_or("")
        .to_string();
    let mut ids = match plugin.format {
        FlPluginFormat::Clap => {
            let mut ids: Vec<String> = plugin.clap_id.iter().map(|c| format!("clap:{c}")).collect();
            ids.push(format!("clap:{}", id(&plugin.name)));
            ids
        }
        FlPluginFormat::Vst2 | FlPluginFormat::Vst3 => {
            vec![
                format!("vst3:{}", id(&plugin.name)),
                format!("vst3:{}", id(&stem)),
            ]
        }
    };
    ids.dedup();
    ids.retain(|i| !i.ends_with(':'));
    ids
}

/// A slot for someone else's plug-in, and what is left to find out about it.
/// Its settings come when FL kept them (a VST3's state); one that comes
/// without them is put in switched off, rather than playing its defaults
/// where FL played something else.
fn external_slot(
    plugin: FlWrappedPlugin,
    track_id: &str,
    place: String,
    enabled: bool,
    wet: f32,
    plays_track: bool,
    report: &mut FlBuildReport,
) -> (crate::track::PluginSlot, ExternalPlugin) {
    let candidates = plugin_ids(&plugin);
    let carried = !plugin.state.is_empty();
    if !carried {
        report.plugins_without_settings.push(match plugin.format {
            FlPluginFormat::Vst2 => format!("{place} (VST2 in FL)"),
            _ => place.clone(),
        });
    }
    let slot_id = uuid::Uuid::new_v4().to_string();
    (
        crate::track::PluginSlot {
            id: slot_id.clone(),
            plugin_id: candidates.first().cloned().unwrap_or_default(),
            enabled: enabled && carried,
            state: None,
            sidechain_source: None,
            wet,
        },
        ExternalPlugin {
            track_id: track_id.to_string(),
            slot_id,
            place,
            plugin,
            candidates,
            plays_track,
        },
    )
}

/// The built-in EQ, which FL's Parametric EQ 2 comes across as.
pub const NATIVE_EQ: &str = "hardwave.native.eq";

/// A built-in effect standing in for one of FL's. Its slot is on the track
/// already; its state is made where the plug-ins live, from a fresh one with
/// these parameters set (by id, as automation addresses them).
#[derive(Debug, Clone, PartialEq)]
pub struct NativeEffect {
    pub slot_id: String,
    pub plugin_id: &'static str,
    pub params: Vec<(u32, f64)>,
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
        // In FL's Resample mode the clip plays as a tape, as it does here
        // with resample on: raising the pitch plays it faster, and playing
        // it longer lowers it. The knobs keep FL's values.
        clip.stretch_ratio = length.clamp(0.1, 10.0);
        clip.pitch_semitones = self.pitch_semitones.clamp(-48.0, 48.0);
        clip.treatment.resample = self.resample;
        let stretch = clip.time_factor();

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
    /// Automation clips that came across.
    pub automation_clips: usize,
    /// Automation clips whose knob has nothing here to move, by FL's name
    /// for them ("Build Snare - Channel pitch").
    pub automation_left_behind: Vec<String>,
    /// Some automation that came across bends between its points in FL;
    /// here it runs straight.
    pub automation_curves_straightened: bool,
    /// Channels using FL's quick fade knobs on their sample, which do not
    /// come across yet.
    pub sample_fades_left_behind: Vec<String>,
    /// Playlist tracks holding samples from channels routed to different
    /// inserts, where a track here plays into one. (Channels panned,
    /// swapped or inverted differently each keep theirs on their clips.)
    pub mixed_lanes: Vec<String>,
    /// FL mixer inserts that came across as tracks.
    pub mixer_inserts: usize,
    /// Effects in the mixer that could not come across, as "insert: effect".
    pub effects_left_behind: Vec<String>,
    /// Inserts whose sound went nowhere in FL (every route at zero, as for
    /// a sidechain only): muted here, since a track always plays out.
    pub silent_inserts: Vec<String>,
    /// Inserts sending on to several places, the main one below 100 %: the
    /// main route plays at full level here.
    pub route_levels_left_behind: Vec<String>,
    /// Other people's plug-ins that came without their settings (a VST2, a
    /// CLAP, or a VST3 FL kept none for): put in switched off.
    pub plugins_without_settings: Vec<String>,
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

/// An insert's pan knob (-6400 to 6400) as -1 to 1.
fn fl_insert_pan(raw: i32) -> f64 {
    (raw as f64 / 6_400.0).clamp(-1.0, 1.0)
}

/// An insert's stereo separation (64 mono, 0 as recorded, -64 fully
/// separated) on our 0 (mono) to 2 (twice as wide) scale.
fn fl_separation(raw: i32) -> f64 {
    (1.0 - raw as f64 / 64.0).clamp(0.0, 2.0)
}

/// The insert a channel plays into; FL's master is 0, and so is "none".
fn channel_insert(channel: &FlChannel) -> u32 {
    channel.settings.mixer_insert.max(0) as u32
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
    let mut external_plugins = Vec::new();
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
    // A soloed playlist track is soloed here too, which silences the rest
    // as FL does and gives it all back with one click. FL soloes by
    // switching every other track off, so the switches then mean nothing.
    let soloed = |fl_track: u32| fl.playlist_tracks_solo.contains(&fl_track);
    let track_off = |fl_track: u32| {
        fl.playlist_tracks_solo.is_empty() && fl.playlist_tracks_off.contains(&fl_track)
    };
    let mut row = 0usize;
    // Each track made, with the FL insert it plays into.
    let mut sources: Vec<(String, u32)> = Vec::new();

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
            sources.push((id.clone(), channel_insert(channel)));
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
                (FlChannelKind::Instrument, _)
                    if channel.plugin_name.as_deref() == Some(WRAPPER)
                        && read_wrapper(&channel.plugin_data).is_some() =>
                {
                    let plugin = read_wrapper(&channel.plugin_data).expect("checked above");
                    let carried = !plugin.state.is_empty();
                    let (slot, external) =
                        external_slot(plugin, &id, name.clone(), true, 1.0, true, &mut report);
                    external_plugins.push(external);
                    if let Some(track) = project.track_mut(&id) {
                        track.inserts.insert(0, slot);
                        // Without its settings it would play its default
                        // sound where FL played the song's.
                        track.muted |= !carried;
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
            if soloed(clip.track_index) {
                if let Some(track) = project.track_mut(track_id) {
                    track.soloed = true;
                }
            }
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
            FlClipContent::Automation { .. } => {}
            FlClipContent::Pattern { .. } => {}
        }
    }
    let mut faded = BTreeSet::new();
    let mut lanes_of_channel: BTreeMap<u32, Vec<(String, f64)>> = BTreeMap::new();
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
        // A lane of one channel is where that channel's volume and pan
        // automation goes.
        if let [only] = channels.iter().collect::<Vec<_>>()[..] {
            let gain = fl
                .channels
                .get(*only as usize)
                .map(|c| fl_audio_clip_gain_db(c.settings.volume))
                .unwrap_or(0.0);
            lanes_of_channel
                .entry(*only)
                .or_default()
                .push((track_id.clone(), gain));
        }

        // Pan, stereo swap and polarity belong to the channel in FL. When
        // the lane's channels agree they go on the track, where a lane of
        // one channel also gets that channel's pan automation. When they
        // do not, each clip carries its own channel's, as FL plays it.
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
        let mut agree = true;
        let mut per_clip = false;
        if let Some(first) = settings.first().copied() {
            if settings.iter().all(|s| *s == first) {
                if let Some(track) = project.track_mut(&track_id) {
                    track.pan = fl_pan(first.0);
                    track.swap_lr = first.1;
                    track.phase_invert = first.2;
                }
            } else {
                per_clip = true;
            }
        }
        // The lane plays into the insert most of its clips' channels go to.
        let mut clips_per_insert: BTreeMap<u32, usize> = BTreeMap::new();
        for clip in &clips {
            if let FlClipContent::AudioSample { channel_index, .. } = &clip.content {
                let insert = fl
                    .channels
                    .get(*channel_index as usize)
                    .map(channel_insert)
                    .unwrap_or(0);
                *clips_per_insert.entry(insert).or_default() += 1;
            }
        }
        if clips_per_insert.len() > 1 {
            agree = false;
        }
        if let Some((&insert, _)) = clips_per_insert
            .iter()
            .max_by_key(|(i, n)| (**n, std::cmp::Reverse(**i)))
        {
            sources.push((track_id.clone(), insert));
        }
        if !agree {
            report.mixed_lanes.push(lane_name.clone());
        }
        if let Some(track) = project.track_mut(&track_id) {
            track.muted |= track_off(fl_track);
            track.soloed = soloed(fl_track);
        }

        for clip in clips {
            let FlClipContent::AudioSample {
                channel_index,
                sample_path,
                start_offset_ms,
                fade_in_ms,
                fade_out_ms,
                volume,
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
            // The clip's own fades, in milliseconds, and its own volume on
            // top of its channel's.
            let length = clip.length_ticks.max(1);
            let ticks =
                |ms: f32| ((ms as f64 / 1000.0 / seconds_per_tick).round() as u64).min(length);
            let fade_in_ticks = ticks(*fade_in_ms);
            let fade_out_ticks = ticks(*fade_out_ms).min(length - fade_in_ticks);
            let own_gain_db = if *volume > 0.0 {
                20.0 * (*volume as f64).log10()
            } else {
                -100.0
            };
            let audio = AudioClip {
                id: clip_id.clone(),
                name: channel_name(*channel_index),
                source_path: file.clone(),
                source_hash: String::new(),
                source_start: 0,
                source_end: 0,
                gain_db: (fl_audio_clip_gain_db(settings.volume) + own_gain_db).max(-100.0),
                fade_in_ticks,
                fade_out_ticks,
                muted: clip.muted || !settings.enabled,
                reversed: settings.reversed,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                fade_in_curve: FadeCurve::default(),
                fade_out_curve: FadeCurve::default(),
                warp_markers: Vec::new(),
                source_file: file,
                treatment: if per_clip {
                    SampleTreatment {
                        pan: fl_pan(settings.pan),
                        swap_stereo: settings.swap_stereo,
                        polarity_inverted: settings.polarity_inverted,
                        ..Default::default()
                    }
                } else {
                    SampleTreatment::default()
                },
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

    let mut mixer = build_mixer(
        fl,
        &mut project,
        &mut row,
        &sources,
        &mut external_plugins,
        &mut report,
    );
    let places = AutomationPlaces {
        instruments: &instrument_track,
        lanes: &lanes_of_channel,
        external: &external_plugins,
    };
    build_automation(fl, &mut project, &places, &mut mixer, &mut report);

    report.missing_samples = missing.into_iter().collect();
    FlBuild {
        project,
        audio_fixups,
        sampler_samples,
        native_effects: mixer.native,
        external_plugins,
        report,
    }
}

/// Where the channels went, for their automation to find them.
struct AutomationPlaces<'a> {
    /// A channel's instrument track.
    instruments: &'a BTreeMap<u32, String>,
    /// The audio tracks holding only a channel's samples, with the gain its
    /// volume knob gave their clips.
    lanes: &'a BTreeMap<u32, Vec<(String, f64)>>,
    external: &'a [ExternalPlugin],
}

/// How an FL knob's 0..1 becomes ours.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Scale {
    /// The same 0..1: pan, and other people's plug-ins, whose parameters FL
    /// keeps as the plug-in's own 0..1.
    Same,
    /// A fader to our -60..+6 dB: FL's knob runs 0 to `top` on its own
    /// curve; `offset_db` is added after.
    Fader { top: f64, offset_db: f64 },
    /// A sample channel's volume as its audio track's fader, around the gain
    /// its clips already have.
    ClipGain { baseline_db: f64 },
    /// A Parametric EQ 2 band's gain (+-18 dB) as the built-in EQ's (+-24).
    EqGain,
    /// A Parametric EQ 2 band's frequency (20 Hz to 20 kHz evenly in
    /// octaves) as the built-in EQ's (20 Hz to 20 kHz, linear).
    EqFreq,
}

impl Scale {
    fn apply(self, v: f64) -> f64 {
        let fader = |db: f64| ((db + 60.0) / 66.0).clamp(0.0, 1.0);
        let v = v.clamp(0.0, 1.0);
        match self {
            Scale::Same => v,
            Scale::Fader { top, offset_db } => fader(fl_volume_db((v * top) as u32) + offset_db),
            Scale::ClipGain { baseline_db } => {
                fader(fl_audio_clip_gain_db((v * 12_800.0) as u32) - baseline_db)
            }
            Scale::EqGain => (((v * 2.0 - 1.0) * 18.0 + 24.0) / 48.0).clamp(0.0, 1.0),
            Scale::EqFreq => ((20.0 * 1000f64.powf(v) - 20.0) / 19_980.0).clamp(0.0, 1.0),
        }
    }
}

/// FL's automation clips, as automation clips on the tracks whose knobs they
/// move. A clip moving something that has no counterpart here is named in
/// the report. The points come across where they are; FL's curves between
/// them run straight here.
fn build_automation(
    fl: &FlProject,
    project: &mut Project,
    places: &AutomationPlaces,
    mixer: &mut MixerMap,
    report: &mut FlBuildReport,
) {
    use crate::automation::{AutomationTarget, CurveMode};
    use crate::fl_import::FlAutomationTarget as T;

    // Each automation channel's knob, as (track, target, scale) here: one
    // per track it reaches (a sample channel can be on several lanes).
    let mut resolved: BTreeMap<u32, Vec<(String, AutomationTarget, Scale)>> = BTreeMap::new();
    for link in &fl.automation_links {
        let mut found = Vec::new();
        let instrument_plugin = |channel: u32| {
            let track = places.instruments.get(&channel)?;
            places
                .external
                .iter()
                .find(|e| e.plays_track && &e.track_id == track && !e.plugin.state.is_empty())
                .map(|e| (track.clone(), e.slot_id.clone(), e.plugin.param_ids.clone()))
        };
        match link.target {
            T::InsertVolume(insert) => {
                if let Some(track) = mixer.tracks.get(&insert) {
                    let offset_db = mixer.fader_offset_db.get(&insert).copied().unwrap_or(0.0);
                    found.push((
                        track.clone(),
                        AutomationTarget::TrackVolume,
                        Scale::Fader {
                            top: 16_000.0,
                            offset_db,
                        },
                    ));
                }
            }
            T::InsertPan(insert) => {
                if let Some(track) = mixer.tracks.get(&insert) {
                    found.push((track.clone(), AutomationTarget::TrackPan, Scale::Same));
                }
            }
            T::ChannelVolume(channel) | T::ChannelPan(channel) => {
                let volume = matches!(link.target, T::ChannelVolume(_));
                if let Some(track) = places.instruments.get(&channel) {
                    found.push(if volume {
                        (
                            track.clone(),
                            AutomationTarget::TrackVolume,
                            Scale::Fader {
                                top: 12_800.0,
                                offset_db: 0.0,
                            },
                        )
                    } else {
                        (track.clone(), AutomationTarget::TrackPan, Scale::Same)
                    });
                }
                for (track, gain) in places.lanes.get(&channel).into_iter().flatten() {
                    found.push(if volume {
                        (
                            track.clone(),
                            AutomationTarget::TrackVolume,
                            Scale::ClipGain { baseline_db: *gain },
                        )
                    } else {
                        (track.clone(), AutomationTarget::TrackPan, Scale::Same)
                    });
                }
            }
            T::ChannelPlugin { channel, param } => {
                if let Some((track, slot_id, ids)) = instrument_plugin(channel) {
                    if let Some(param_id) = ids.get(param as usize) {
                        found.push((
                            track,
                            AutomationTarget::PluginParam {
                                slot_id,
                                param_id: *param_id,
                            },
                            Scale::Same,
                        ));
                    }
                }
            }
            T::SlotPlugin {
                insert,
                slot,
                param,
            } => {
                let track = mixer.tracks.get(&insert).cloned();
                match (track, mixer.slots.get(&(insert, slot)).cloned()) {
                    (Some(track), Some((slot_id, SlotKind::Plugin(ids)))) => {
                        if let Some(param_id) = ids.get(param as usize) {
                            found.push((
                                track,
                                AutomationTarget::PluginParam {
                                    slot_id,
                                    param_id: *param_id,
                                },
                                Scale::Same,
                            ));
                        }
                    }
                    // Parametric EQ 2 numbers its gains 0 to 6 and its
                    // frequencies 7 to 13; the built-in EQ has four
                    // parameters a band. An automated band is switched on,
                    // whatever its gain at the start.
                    (Some(track), Some((slot_id, SlotKind::Eq))) if param < 14 => {
                        let band = param % 7;
                        let (param_id, scale) = if param < 7 {
                            (band * 4 + 2, Scale::EqGain)
                        } else {
                            (band * 4 + 1, Scale::EqFreq)
                        };
                        if let Some(eq) = mixer.native.iter_mut().find(|e| e.slot_id == slot_id) {
                            eq.params.push((band * 4, 1.0));
                        }
                        found.push((
                            track,
                            AutomationTarget::PluginParam { slot_id, param_id },
                            scale,
                        ));
                    }
                    _ => {}
                }
            }
            T::ChannelPitch(_)
            | T::InsertSeparation(_)
            | T::SlotEnabled { .. }
            | T::SlotMix { .. }
            | T::Other => {}
        }
        resolved.entry(link.channel).or_default().extend(found);
    }

    let mut left_behind = BTreeSet::new();
    for clip in &fl.playlist_clips {
        let FlClipContent::Automation {
            channel_index,
            target: name,
            offset_ticks,
        } = &clip.content
        else {
            continue;
        };
        // Muted, or on a track switched off: it moves nothing in FL either.
        let switched_off = fl.playlist_tracks_solo.is_empty()
            && fl.playlist_tracks_off.contains(&clip.track_index);
        if clip.muted || switched_off {
            continue;
        }
        let targets = resolved.get(channel_index).filter(|t| !t.is_empty());
        let points = fl
            .channels
            .get(*channel_index as usize)
            .map(|c| c.automation.as_slice())
            .unwrap_or(&[]);
        let (Some(targets), false) = (targets, points.is_empty()) else {
            if left_behind.insert(name.clone()) {
                report.automation_left_behind.push(name.clone());
            }
            continue;
        };
        let length = clip.length_ticks.max(1);
        let window = automation_window(points, *offset_ticks, length);
        report.automation_curves_straightened |= points.iter().any(|p| p.tension.abs() > 0.01);
        for (track_id, target, scale) in targets {
            let mut automation = crate::automation_clip::AutomationClip::new(
                uuid::Uuid::new_v4().to_string(),
                target.clone(),
                clip.start_tick,
                length,
            );
            for (tick, value) in &window {
                automation.insert_point(*tick, scale.apply(*value), CurveMode::Linear);
            }
            if let Some(track) = project.track_mut(track_id) {
                track.automation_clips.push(automation);
                report.automation_clips += 1;
            }
        }
    }
}

/// The part of an automation clip's points a placement plays: `length`
/// ticks from `from`, as points from 0, with the value where it starts and
/// where it ends. Between points the value runs straight; before the first
/// and after the last it holds. Two points at one place (FL's jump) come a
/// tick apart, so the jump is kept.
fn automation_window(
    points: &[crate::fl_import::FlAutomationPoint],
    from: u64,
    length: u64,
) -> Vec<(u64, f64)> {
    let value_at = |tick: u64| -> f64 {
        let after = points.iter().position(|p| p.tick > tick);
        match after {
            None => points.last().map(|p| p.value).unwrap_or(0.0),
            Some(0) => points[0].value,
            Some(i) => {
                let (a, b) = (points[i - 1], points[i]);
                let span = (b.tick - a.tick).max(1) as f64;
                a.value + (b.value - a.value) * (tick - a.tick) as f64 / span
            }
        }
    };
    let to = from + length;
    let mut out = vec![(0, value_at(from))];
    for p in points.iter().filter(|p| p.tick > from && p.tick < to) {
        let tick = (p.tick - from).max(out.last().map(|(t, _)| t + 1).unwrap_or(0));
        out.push((tick, p.value));
    }
    let end = value_at(to);
    if out.last().map(|(t, _)| *t < length).unwrap_or(true) {
        out.push((length, end));
    }
    out
}

/// FL's mixer. Every track made so far plays into the insert its channel
/// was routed to; the inserts that sound reaches become tracks of their own
/// after the playlist's rows, with FL's fader, pan and switches, and send on
/// where FL sent them. Insert 0 is the master.
fn build_mixer(
    fl: &FlProject,
    project: &mut Project,
    row: &mut usize,
    sources: &[(String, u32)],
    external: &mut Vec<ExternalPlugin>,
    report: &mut FlBuildReport,
) -> MixerMap {
    let mut map = MixerMap::default();
    let inserts: BTreeMap<u32, &FlMixerTrack> = fl.mixer.iter().map(|m| (m.index, m)).collect();
    // The routes that carry sound: FL makes a route at zero for a sidechain.
    let live = |m: &FlMixerTrack| -> Vec<FlRoute> {
        m.routes
            .iter()
            .filter(|r| {
                r.level > 0 && r.to != m.index && (r.to == 0 || inserts.contains_key(&r.to))
            })
            .copied()
            .collect()
    };
    let mut reached = BTreeSet::new();
    let mut queue: Vec<u32> = sources.iter().map(|(_, insert)| *insert).collect();
    while let Some(insert) = queue.pop() {
        let Some(m) = inserts.get(&insert) else {
            continue;
        };
        if insert != 0 && reached.insert(insert) {
            queue.extend(live(m).iter().map(|r| r.to));
        }
    }

    let name_of = |m: &FlMixerTrack| -> String {
        if m.name.is_empty() {
            format!("Insert {}", m.index)
        } else {
            m.name.clone()
        }
    };
    let mut insert_track: BTreeMap<u32, String> = BTreeMap::new();
    for insert in &reached {
        *row += 1;
        let id = project.claim_row(*row, &name_of(inserts[insert]), false);
        insert_track.insert(*insert, id);
    }
    report.mixer_inserts = insert_track.len();
    let track_of = |insert: u32| insert_track.get(&insert).cloned();

    for (track_id, insert) in sources {
        if let Some(track) = project.track_mut(track_id) {
            track.output_bus = track_of(*insert);
        }
    }

    // Inserts something plays its whole output into are groups; the ones
    // only sent to are returns.
    let mut fed: BTreeSet<u32> = sources.iter().map(|(_, insert)| *insert).collect();
    let levels = |track: &mut crate::track::Track, m: &FlMixerTrack| {
        track.volume_db = fl_volume_db(m.volume.max(0) as u32);
        track.pan = fl_insert_pan(m.pan);
        track.stereo_separation = fl_separation(m.stereo_separation);
        track.muted = !m.enabled;
        track.soloed = m.solo;
        track.phase_invert = m.polarity_inverted;
        track.swap_lr = m.swap_stereo;
    };
    if let Some(master) = inserts.get(&0) {
        if let Some(track) = project
            .tracks
            .iter_mut()
            .find(|t| matches!(t.kind, TrackKind::Master))
        {
            levels(track, master);
            let id = track.id.clone();
            map.tracks.insert(0, id.clone());
            track.inserts = effects(master, "Master", &id, &mut map, external, report);
        }
    }
    for (insert, track_id) in &insert_track {
        let m = inserts[insert];
        let name = name_of(m);
        let routes = live(m);
        let main = if routes.iter().any(|r| r.to == 0) {
            None
        } else {
            routes
                .iter()
                .find(|r| r.level == 12_800)
                .or(routes.first())
                .copied()
        };
        if let Some(main) = main {
            fed.insert(main.to);
        }
        let Some(track) = project.track_mut(track_id) else {
            continue;
        };
        levels(track, m);
        if routes.is_empty() {
            track.muted = true;
            report.silent_inserts.push(name.clone());
        }
        if let Some(main) = main {
            track.output_bus = track_of(main.to);
            if main.level != 12_800 {
                if routes.len() == 1 {
                    // Its only route comes after its fader: the same as a
                    // fader that much lower.
                    let offset = fl_volume_db(main.level as u32);
                    track.volume_db = (track.volume_db + offset).max(-100.0);
                    map.fader_offset_db.insert(*insert, offset);
                } else {
                    report.route_levels_left_behind.push(name.clone());
                }
            }
        }
        track.sends = routes
            .iter()
            .filter(|r| r.to != 0 && Some(**r) != main)
            .filter_map(|r| {
                Some(crate::mixer::Send {
                    target: track_of(r.to)?,
                    gain_db: fl_volume_db(r.level as u32),
                    pre_fader: false,
                    enabled: true,
                })
            })
            .collect();
        track.inserts = effects(m, &name, track_id, &mut map, external, report);
    }
    for (insert, track_id) in &insert_track {
        if let Some(track) = project.track_mut(track_id) {
            track.kind = if fed.contains(insert) {
                TrackKind::Bus
            } else {
                TrackKind::Return
            };
        }
    }
    map.tracks.extend(insert_track);
    map
}

/// What the mixer made, for automation to find its way to.
#[derive(Default)]
struct MixerMap {
    native: Vec<NativeEffect>,
    /// Each FL insert's track; 0 is the master's.
    tracks: BTreeMap<u32, String>,
    /// How far a track's fader sits from FL's, where an insert's one route
    /// at less than full level was folded into it.
    fader_offset_db: BTreeMap<u32, f64>,
    /// An insert's FL slot, as (insert, slot), to our slot and what is in it.
    slots: BTreeMap<(u32, u32), (String, SlotKind)>,
}

/// What a slot holds, as automation needs to know it.
#[derive(Debug, Clone)]
enum SlotKind {
    /// Someone else's VST3 with its settings: its parameters' ids in FL's
    /// order.
    Plugin(Vec<u32>),
    /// The built-in EQ standing in for FL's Parametric EQ 2.
    Eq,
}

/// An insert's effects as slots, in FL's order: FL's own as their built-in
/// counterparts where there is one, other people's as themselves. The rest
/// are named in the report.
fn effects(
    m: &FlMixerTrack,
    name: &str,
    track_id: &str,
    map: &mut MixerMap,
    external: &mut Vec<ExternalPlugin>,
    report: &mut FlBuildReport,
) -> Vec<crate::track::PluginSlot> {
    let mut slots = Vec::new();
    for slot in &m.slots {
        let fl_slot = slot.index;
        let wet = (slot.mix as f32 / 12_800.0).clamp(0.0, 1.0);
        let wrapped = (slot.plugin == WRAPPER)
            .then(|| read_wrapper(&slot.data))
            .flatten();
        if let Some(plugin) = wrapped {
            let place = format!("{name}: {}", slot.label());
            let enabled = slot.enabled && m.effects_enabled;
            let kind =
                (!plugin.state.is_empty()).then(|| SlotKind::Plugin(plugin.param_ids.clone()));
            let (slot, plugin) =
                external_slot(plugin, track_id, place, enabled, wet, false, report);
            if let Some(kind) = kind {
                map.slots
                    .insert((m.index, fl_slot), (slot.id.clone(), kind));
            }
            slots.push(slot);
            external.push(plugin);
            continue;
        }
        let Some((plugin_id, params)) = native_counterpart(slot) else {
            report
                .effects_left_behind
                .push(format!("{name}: {}", slot.label()));
            continue;
        };
        let slot_id = uuid::Uuid::new_v4().to_string();
        slots.push(crate::track::PluginSlot {
            id: slot_id.clone(),
            plugin_id: plugin_id.to_string(),
            enabled: slot.enabled && m.effects_enabled,
            state: None,
            sidechain_source: None,
            wet,
        });
        if plugin_id == NATIVE_EQ {
            map.slots
                .insert((m.index, fl_slot), (slot_id.clone(), SlotKind::Eq));
        }
        map.native.push(NativeEffect {
            slot_id,
            plugin_id,
            params,
        });
    }
    if m.eq_gains.iter().any(|g| *g != 0) {
        report
            .effects_left_behind
            .push(format!("{name}: the insert's own EQ"));
    }
    slots
}

/// The built-in effect that stands in for an FL one, with its settings.
fn native_counterpart(
    slot: &crate::fl_import::FlMixerSlot,
) -> Option<(&'static str, Vec<(u32, f64)>)> {
    match slot.plugin.as_str() {
        "Fruity Parametric EQ 2" => Some((NATIVE_EQ, parametric_eq_2(&slot.data)?)),
        _ => None,
    }
}

/// FL's Parametric EQ 2 as the built-in EQ's parameters.
///
/// Its data is a version, then for each of the seven bands its gain (in
/// hundredths of a dB), frequency, width, type and slope, then the main
/// level. The frequency runs 20 Hz to 20 kHz evenly in octaves over 0 to
/// 65536. FL's widths are its own measure; the Q for a bell or shelf follows
/// the reading DawVert uses, and a cut is a plain 24 dB one, ours.
fn parametric_eq_2(data: &[u8]) -> Option<Vec<(u32, f64)>> {
    // The built-in EQ's parameters: four per band, then the output gain,
    // then each band's shape.
    const OUTPUT_GAIN: u32 = 28;
    const SHAPE: u32 = 29;
    const LOW_CUT: f64 = 0.0;
    const LOW_SHELF: f64 = 1.0;
    const BELL: f64 = 2.0;
    const NOTCH: f64 = 3.0;
    const HIGH_SHELF: f64 = 4.0;
    const HIGH_CUT: f64 = 5.0;

    let words: Vec<i32> = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|w| i32::from_le_bytes(*w))
        .collect();
    if words.len() < 37 {
        return None;
    }
    let bell_q = |width: f64| 1.0 / ((width + 0.01) * 3.0);
    let shelf_q = |width: f64| (1.0 - width) * 1.2;
    let mut params = Vec::new();
    for band in 0..7 {
        let gain = words[1 + band] as f64 / 100.0;
        let freq = 20.0 * 1000f64.powf(words[8 + band] as f64 / 65_536.0);
        let width = words[15 + band] as f64 / 65_536.0;
        // 0 is off; 2, a band pass, has no counterpart here.
        let (shape, q, on) = match words[22 + band] {
            1 => (HIGH_CUT, std::f64::consts::FRAC_1_SQRT_2, true),
            3 => (LOW_CUT, std::f64::consts::FRAC_1_SQRT_2, true),
            4 => (NOTCH, bell_q(width), true),
            5 => (LOW_SHELF, shelf_q(width), gain != 0.0),
            6 => (BELL, bell_q(width), gain != 0.0),
            7 => (HIGH_SHELF, shelf_q(width), gain != 0.0),
            _ => (BELL, 1.0, false),
        };
        let base = band as u32 * 4;
        params.extend([
            (base, if on { 1.0 } else { 0.0 }),
            (base + 1, freq.clamp(20.0, 20_000.0)),
            (base + 2, gain.clamp(-24.0, 24.0)),
            (base + 3, q.clamp(0.1, 10.0)),
            (SHAPE + band as u32, shape),
        ]);
    }
    params.push((OUTPUT_GAIN, (words[36] as f64 / 100.0).clamp(-24.0, 24.0)));
    Some(params)
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
            plugin_data: Vec::new(),
            automation: Vec::new(),
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
                        fade_in_ms: 0.0,
                        fade_out_ms: 0.0,
                        volume: 1.0,
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
                        fade_in_ms: 0.0,
                        fade_out_ms: 0.0,
                        volume: 1.0,
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
                        fade_in_ms: 0.0,
                        fade_out_ms: 0.0,
                        volume: 1.0,
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
                        offset_ticks: 0,
                    },
                    muted: false,
                },
            ],
            mixer: Vec::new(),
            playlist_track_names: vec![(9, "Vocals".into())],
            playlist_tracks_off: Vec::new(),
            automation_links: Vec::new(),
            playlist_tracks_solo: Vec::new(),
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
        // The automation clip moves nothing FL told us about.
        assert_eq!(built.report.automation_clips, 0);
        assert_eq!(
            built.report.automation_left_behind,
            vec!["Cutoff".to_string()]
        );
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
        fl.channels[4].settings.polarity_inverted = true;
        fl.channels[4].settings.mixer_insert = 3;
        fl.playlist_clips[3].track_index = 5;
        let built = build_project(&fl, "Industrial", &here);
        assert_eq!(built.report.mixed_lanes, vec!["Track 5".to_string()]);
        // The track stays centred; each clip pans as its own channel.
        let lane = built.project.track("insert-003").unwrap();
        assert_eq!(lane.pan, 0.0);
        assert!(!lane.phase_invert);
        let treatments: Vec<_> = lane
            .clips
            .iter()
            .filter_map(|c| match &c.content {
                ClipContent::Audio(a) => Some((a.name.clone(), a.treatment)),
                _ => None,
            })
            .collect();
        assert!(treatments
            .iter()
            .any(|(_, t)| t.pan == 0.5 && !t.polarity_inverted));
        assert!(treatments
            .iter()
            .any(|(_, t)| t.pan == -1.0 && t.polarity_inverted));
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
            treatment: Default::default(),
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

        assert!(!clip.treatment.resample);

        // The same in Resample: a tape, twice as long and so an octave down,
        // with FL's knobs as they were.
        let mut tape = blank_clip();
        AudioFixup {
            fit_seconds: Some(4.0),
            resample: true,
            ..fixup()
        }
        .apply(&mut tape, 96_000, 48_000);
        assert!(tape.treatment.resample);
        assert!((tape.stretch_ratio - 2.0).abs() < 1e-9);
        assert_eq!(tape.pitch_semitones, 0.0);
        assert!((tape.time_factor() - 2.0).abs() < 1e-9);

        // Resample with only the pitch knob: up an octave plays twice as fast.
        let mut up = blank_clip();
        AudioFixup {
            resample: true,
            pitch_semitones: 12.0,
            ..fixup()
        }
        .apply(&mut up, 96_000, 48_000);
        assert_eq!(up.stretch_ratio, 1.0);
        assert!((up.pitch_semitones - 12.0).abs() < 1e-9);
        assert!((up.time_factor() - 0.5).abs() < 1e-9);
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
    fn a_tape_s_offsets_count_in_the_file_at_its_speed() {
        // Up an octave in Resample the sample plays twice as fast: a
        // quarter second into it is half a second into the file.
        let mut clip = blank_clip();
        AudioFixup {
            start_ms: 250.0,
            resample: true,
            pitch_semitones: 12.0,
            ..fixup()
        }
        .apply(&mut clip, 96_000, 48_000);
        assert_eq!(clip.source_start, 24_000);
        assert_eq!(clip.source_end, 96_000, "the rest of the file");
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

    fn mixer_song() -> FlProject {
        let mut fl = song();
        // The screech plays into insert 1, the kick render into 2, the 808
        // into 3 and the lost vocal into 7.
        fl.channels[0].settings.mixer_insert = 1;
        fl.channels[1].settings.mixer_insert = 2;
        fl.channels[2].settings.mixer_insert = 3;
        fl.channels[4].settings.mixer_insert = 7;
        let insert = |index: u32, name: &str, routes: Vec<FlRoute>| FlMixerTrack {
            index,
            name: name.into(),
            routes,
            ..Default::default()
        };
        let to = |to, level| FlRoute { to, level };
        fl.mixer = vec![
            FlMixerTrack {
                volume: 10_000,
                ..insert(0, "", vec![])
            },
            // Into the group only: the route to the master is at zero.
            FlMixerTrack {
                volume: 10_000,
                pan: 3_200,
                stereo_separation: 64,
                polarity_inverted: true,
                ..insert(1, "Screech", vec![to(0, 0), to(4, 12_800)])
            },
            // To the master, and sent to the reverb at half.
            insert(2, "", vec![to(0, 12_800), to(5, 6_400)]),
            // Its one route at half: the same as its fader that much lower.
            insert(3, "808", vec![to(4, 6_400)]),
            insert(4, "Group", vec![to(0, 12_800)]),
            insert(5, "Reverb", vec![to(0, 12_800)]),
            // Nothing plays into it.
            insert(6, "Unused", vec![to(0, 12_800)]),
            // Sends nowhere: a sidechain source only.
            insert(7, "Ghost", vec![to(0, 0)]),
        ];
        fl
    }

    #[test]
    fn the_mixer_s_inserts_become_tracks_routed_as_in_fl() {
        let built = build_project(&mixer_song(), "Mixer", &here);
        let p = &built.project;
        let by_name = |name: &str| {
            p.tracks
                .iter()
                .find(|t| t.name == name && matches!(t.kind, TrackKind::Bus | TrackKind::Return))
                .unwrap_or_else(|| panic!("no insert track {name}"))
        };
        let screech = by_name("Screech");
        let group = by_name("Group");
        let reverb = by_name("Reverb");
        let insert_2 = by_name("Insert 2");
        let eight = by_name("808");
        let ghost = by_name("Ghost");
        assert!(
            p.tracks.iter().all(|t| t.name != "Unused"),
            "nothing reaches it"
        );
        assert_eq!(built.report.mixer_inserts, 6);

        // The tracks play into their channels' inserts.
        assert_eq!(
            p.track("insert-001").unwrap().output_bus.as_ref(),
            Some(&screech.id)
        );
        assert_eq!(
            p.track("insert-002").unwrap().output_bus.as_ref(),
            Some(&eight.id)
        );
        assert_eq!(
            p.track("insert-003").unwrap().output_bus.as_ref(),
            Some(&insert_2.id)
        );
        assert_eq!(
            p.track("insert-004").unwrap().output_bus.as_ref(),
            Some(&ghost.id)
        );

        assert_eq!(screech.output_bus.as_ref(), Some(&group.id));
        assert!(screech.sends.is_empty(), "the silent route is no send");
        assert!((screech.volume_db - fl_volume_db(10_000)).abs() < 1e-9);
        assert_eq!(screech.pan, 0.5);
        assert_eq!(screech.stereo_separation, 0.0);
        assert!(screech.phase_invert);

        assert_eq!(insert_2.output_bus, None);
        assert_eq!(insert_2.sends.len(), 1);
        assert_eq!(insert_2.sends[0].target, reverb.id);
        assert!((insert_2.sends[0].gain_db - fl_volume_db(6_400)).abs() < 1e-9);
        assert!(!insert_2.sends[0].pre_fader);

        assert_eq!(eight.output_bus.as_ref(), Some(&group.id));
        assert!((eight.volume_db - fl_volume_db(6_400)).abs() < 1e-9);

        assert!(matches!(group.kind, TrackKind::Bus));
        assert!(matches!(reverb.kind, TrackKind::Return), "only sent to");
        assert!(ghost.muted);
        assert_eq!(built.report.silent_inserts, vec!["Ghost".to_string()]);

        let master = p
            .tracks
            .iter()
            .find(|t| matches!(t.kind, TrackKind::Master))
            .unwrap();
        assert!((master.volume_db - fl_volume_db(10_000)).abs() < 1e-9);
    }

    /// A Parametric EQ 2's data: every band a bell at 0 dB in the middle of
    /// the range at FL's default width, then `set` on top.
    fn parametric_eq_2_data(set: &[(usize, i32)]) -> Vec<u8> {
        let mut words = vec![0i32; 88];
        words[0] = 8;
        for band in 0..7 {
            words[8 + band] = 32_768;
            words[15 + band] = 25_412;
            words[22 + band] = 6;
        }
        for (at, value) in set {
            words[*at] = *value;
        }
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn parametric_eq_2_comes_across_as_the_built_in_eq() {
        let mut fl = mixer_song();
        let slot = |index, plugin: &str, data| crate::fl_import::FlMixerSlot {
            index,
            plugin: plugin.into(),
            name: None,
            enabled: true,
            mix: 12_800,
            data,
        };
        // Band 4 up 6 dB, band 1 a low cut, band 7 off; the main level down 3.
        let eq = parametric_eq_2_data(&[(4, 600), (22, 3), (28, 0), (36, -300)]);
        fl.mixer[0].slots = vec![
            crate::fl_import::FlMixerSlot {
                mix: 6_400,
                ..slot(0, "Fruity Parametric EQ 2", eq.clone())
            },
            slot(3, "Maximus", Vec::new()),
        ];
        fl.mixer[4].slots = vec![slot(0, "Fruity Parametric EQ 2", eq)];
        fl.mixer[4].effects_enabled = false;
        fl.mixer[4].eq_gains = [0, 150, 0];
        let built = build_project(&fl, "Mixer", &here);
        let p = &built.project;

        let master = p
            .tracks
            .iter()
            .find(|t| matches!(t.kind, TrackKind::Master))
            .unwrap();
        assert_eq!(
            master.inserts.len(),
            1,
            "Maximus has nothing to stand in for it"
        );
        assert_eq!(master.inserts[0].plugin_id, NATIVE_EQ);
        assert!(master.inserts[0].enabled);
        assert_eq!(master.inserts[0].wet, 0.5);
        let group = p.tracks.iter().find(|t| t.name == "Group").unwrap();
        assert!(
            !group.inserts[0].enabled,
            "the insert's effects are off in FL"
        );
        assert_eq!(
            built.report.effects_left_behind,
            vec![
                "Master: Maximus".to_string(),
                "Group: the insert's own EQ".into()
            ]
        );

        let effect = built
            .native_effects
            .iter()
            .find(|e| e.slot_id == master.inserts[0].id)
            .unwrap();
        let value = |id: u32| {
            effect
                .params
                .iter()
                .rev()
                .find(|(p, _)| *p == id)
                .map(|(_, v)| *v)
                .unwrap()
        };
        // Band 4: on, 6 dB, at 632 Hz (halfway in octaves from 20 Hz to 20 kHz).
        assert_eq!(value(12), 1.0);
        assert_eq!(value(14), 6.0);
        assert!((value(13) - 632.46).abs() < 0.01);
        assert_eq!(value(29 + 3), 2.0, "a bell");
        // Band 1: a cut is on whatever its gain.
        assert_eq!(value(0), 1.0);
        assert_eq!(value(29), 0.0, "a low cut");
        // Band 2: a bell at 0 dB does nothing, so it stays off.
        assert_eq!(value(4), 0.0);
        // Band 7: off in FL.
        assert_eq!(value(24), 0.0);
        assert_eq!(value(28), -3.0, "the main level");
    }

    #[test]
    fn other_people_s_plug_ins_come_with_their_settings() {
        use crate::fl_import::{test_vst3_wrapper, FlMixerSlot};
        let mut fl = mixer_song();
        // The screech plays Serum2, with its patch; the 808 is a VST2
        // instrument now, whose patch cannot come.
        let screech = &mut fl.channels[0];
        screech.plugin_data = test_vst3_wrapper("Serum2", true, b"the patch");
        let vst2 = crate::fl_import::test_wrapper(&[
            (50, &4u32.to_le_bytes()),
            (54, b"Serum"),
            (55, b"C:\\VstPlugins\\Serum_x64.dll"),
        ]);
        fl.channels[2] = channel("808", FlChannelKind::Instrument, None, Some(WRAPPER));
        fl.channels[2].plugin_data = vst2;
        fl.channels[2].settings.mixer_insert = 3;
        // The group has Pro-Q 4 with its settings, at half mix.
        fl.mixer[4].slots = vec![FlMixerSlot {
            index: 2,
            plugin: WRAPPER.into(),
            name: None,
            enabled: true,
            mix: 6_400,
            data: test_vst3_wrapper("FabFilter Pro-Q 4", false, b"bands"),
        }];
        let built = build_project(&fl, "Plug-ins", &here);
        let p = &built.project;

        let serum = p.track("insert-001").unwrap();
        assert!(!serum.muted, "its sound comes with it");
        assert_eq!(serum.inserts[0].plugin_id, "vst3:serum2");
        assert!(serum.inserts[0].enabled);
        let serum_plugin = built
            .external_plugins
            .iter()
            .find(|e| e.slot_id == serum.inserts[0].id)
            .unwrap();
        assert!(serum_plugin.plays_track);
        assert_eq!(serum_plugin.plugin.state, b"the patch");

        let eight = p.track("insert-002").unwrap();
        assert!(
            eight.muted,
            "a VST2's patch cannot come, so it would play the wrong sound"
        );
        assert_eq!(
            eight.inserts[0].plugin_id, "vst3:serum",
            "looked for as its VST3"
        );
        assert!(!eight.inserts[0].enabled);
        assert_eq!(
            built.report.plugins_without_settings,
            vec!["808 (VST2 in FL)".to_string()]
        );
        assert!(built.report.plugins_left_behind.is_empty());

        let group = p.tracks.iter().find(|t| t.name == "Group").unwrap();
        assert_eq!(group.inserts[0].plugin_id, "vst3:fabfilter-pro-q-4");
        assert_eq!(group.inserts[0].wet, 0.5);
        let pro_q = built
            .external_plugins
            .iter()
            .find(|e| e.slot_id == group.inserts[0].id)
            .unwrap();
        assert_eq!(pro_q.place, "Group: FabFilter Pro-Q 4");
        assert_eq!(pro_q.candidates, vec!["vst3:fabfilter-pro-q-4".to_string()]);
        assert!(!pro_q.plays_track);
        assert!(built.report.effects_left_behind.is_empty());
    }

    fn point(beats: f64, value: f64) -> crate::fl_import::FlAutomationPoint {
        crate::fl_import::FlAutomationPoint {
            tick: (beats * hardwave_midi::PPQ as f64) as u64,
            value,
            tension: 0.0,
        }
    }

    #[test]
    fn a_placement_plays_its_part_of_the_clip_with_its_jumps() {
        let beat = hardwave_midi::PPQ;
        // Up over four beats, a jump down at four, on to 0.5 at six.
        let points = [
            point(0.0, 0.0),
            point(4.0, 1.0),
            point(4.0, 0.25),
            point(6.0, 0.5),
        ];
        assert_eq!(
            automation_window(&points, 0, 8 * beat),
            vec![
                (0, 0.0),
                (4 * beat, 1.0),
                (4 * beat + 1, 0.25),
                (6 * beat, 0.5),
                (8 * beat, 0.5)
            ],
            "the jump is a tick apart; after the last point it holds"
        );
        // From beat 2, for three beats: it starts half way up and ends
        // half way from the jump to the last point.
        assert_eq!(
            automation_window(&points, 2 * beat, 3 * beat),
            vec![
                (0, 0.5),
                (2 * beat, 1.0),
                (2 * beat + 1, 0.25),
                (3 * beat, 0.375)
            ]
        );
    }

    #[test]
    fn automation_clips_move_what_fl_moved() {
        use crate::automation::AutomationTarget;
        use crate::fl_import::{
            test_vst3_wrapper, FlAutomationLink, FlAutomationTarget as T, FlMixerSlot,
        };
        let beat = hardwave_midi::PPQ;
        let mut fl = mixer_song();
        // The screech plays Serum2; FL lists its parameters' ids as 0, 7
        // and "prog".
        fl.channels[0].plugin_data = test_vst3_wrapper("Serum2", true, b"patch");
        // The group has a Parametric EQ 2 in its first slot.
        fl.mixer[4].slots = vec![FlMixerSlot {
            index: 0,
            plugin: "Fruity Parametric EQ 2".into(),
            name: None,
            enabled: true,
            mix: 12_800,
            data: parametric_eq_2_data(&[]),
        }];
        let rise = vec![point(0.0, 0.0), point(4.0, 1.0)];
        let links = [
            ("Group - Volume", T::InsertVolume(4)),
            (
                "EQ - Group - Band 3 freq",
                T::SlotPlugin {
                    insert: 4,
                    slot: 0,
                    param: 9,
                },
            ),
            ("Screech - Channel pitch", T::ChannelPitch(0)),
            (
                "Serum 2 - Macro 2",
                T::ChannelPlugin {
                    channel: 0,
                    param: 1,
                },
            ),
            ("Kick render - Channel volume", T::ChannelVolume(1)),
        ];
        for (i, (name, target)) in links.iter().enumerate() {
            let index = 5 + i as u32;
            let mut automation = channel(name, FlChannelKind::Automation, None, None);
            automation.automation = rise.clone();
            fl.channels.push(automation);
            fl.automation_links.push(FlAutomationLink {
                channel: index,
                target: *target,
            });
            fl.playlist_clips.push(FlPlaylistClip {
                track_index: 20,
                start_tick: 8 * beat,
                length_ticks: 4 * beat,
                content: FlClipContent::Automation {
                    channel_index: index,
                    target: name.to_string(),
                    offset_ticks: 0,
                },
                muted: false,
            });
        }
        // A muted placement moves nothing, here or in FL.
        let mut muted = fl.playlist_clips.last().unwrap().clone();
        muted.muted = true;
        muted.start_tick = 0;
        fl.playlist_clips.push(muted);

        let built = build_project(&fl, "Automation", &here);
        let p = &built.project;
        let group = p.tracks.iter().find(|t| t.name == "Group").unwrap();
        let fader = &group
            .automation_clips
            .iter()
            .find(|c| c.target == AutomationTarget::TrackVolume)
            .unwrap();
        assert_eq!((fader.start_tick, fader.length_ticks), (8 * beat, 4 * beat));
        let first = fader.lane.points.first().unwrap().value;
        let last = fader.lane.points.last().unwrap().value;
        assert_eq!(
            first, 0.0,
            "FL's fader at the bottom is our -60 dB and below"
        );
        assert!(
            (last - (fl_volume_db(16_000) + 60.0) / 66.0).abs() < 1e-9,
            "the top is FL's +4.7 dB"
        );

        let eq_slot = &group.inserts[0].id;
        let eq_freq = group
            .automation_clips
            .iter()
            .find(|c| matches!(&c.target, AutomationTarget::PluginParam { slot_id, param_id: 9 } if slot_id == eq_slot))
            .expect("band 3's frequency, the built-in EQ's parameter 9");
        let hz = |v: f64| 20.0 + v * 19_980.0;
        assert!((hz(eq_freq.lane.points.last().unwrap().value) - 20_000.0).abs() < 1e-6);
        let eq = built
            .native_effects
            .iter()
            .find(|e| &e.slot_id == eq_slot)
            .unwrap();
        assert_eq!(
            eq.params.last(),
            Some(&(8, 1.0)),
            "the automated band is switched on"
        );

        let serum = p.track("insert-001").unwrap();
        assert!(serum.automation_clips.iter().any(|c| c.target
            == AutomationTarget::PluginParam {
                slot_id: serum.inserts[0].id.clone(),
                param_id: 7,
            }));

        let kicks = p.track("insert-003").unwrap();
        let volume = kicks
            .automation_clips
            .iter()
            .find(|c| c.target == AutomationTarget::TrackVolume)
            .expect("the kick render's lane has only its channel");
        // At the knob's default the lane is where the clips already are.
        let at_default = Scale::ClipGain {
            baseline_db: fl_audio_clip_gain_db(10_000),
        }
        .apply(10_000.0 / 12_800.0);
        assert!((at_default - 60.0 / 66.0).abs() < 1e-3);
        assert_eq!(volume.lane.points.len(), 2);

        assert_eq!(
            built.report.automation_left_behind,
            vec!["Cutoff".to_string(), "Screech - Channel pitch".into()]
        );
        assert_eq!(built.report.automation_clips, 4);
    }

    #[test]
    fn a_track_soloed_in_fl_is_soloed_and_the_rest_are_not_switched_off() {
        let mut fl = song();
        // The kick lane (playlist track 5) soloed: FL switched every other
        // track off for it, the patterns' track 1 among them.
        fl.playlist_tracks_solo = vec![5];
        fl.playlist_tracks_off = vec![1, 9];
        let built = build_project(&fl, "Solo", &here);
        let p = &built.project;
        let kicks = p.track("insert-003").unwrap();
        assert!(kicks.soloed && !kicks.muted);
        let vocals = p.track("insert-004").unwrap();
        assert!(!vocals.muted && !vocals.soloed, "unsoloing gives it back");
        let screech = p.track("insert-001").unwrap();
        assert!(!screech.soloed);
        let ClipContent::Midi(midi) = &screech.clips[0].content else {
            panic!("expected notes");
        };
        assert!(midi.clip.notes.iter().all(|n| !n.muted));
    }

    #[test]
    fn a_clip_s_own_fades_and_volume_come_with_it() {
        let mut fl = song();
        let FlClipContent::AudioSample {
            fade_in_ms,
            fade_out_ms,
            volume,
            ..
        } = &mut fl.playlist_clips[1].content
        else {
            panic!("expected audio");
        };
        // At 160 BPM a beat is 375 ms: a quarter beat in, half a beat out
        // (the clip is a beat long), -4 dB.
        *fade_in_ms = 93.75;
        *fade_out_ms = 187.5;
        *volume = 0.631;
        let built = build_project(&fl, "Fades", &here);
        let kicks = built.project.track("insert-003").unwrap();
        let ClipContent::Audio(clip) = &kicks.clips[0].content else {
            panic!("expected audio");
        };
        let beat = hardwave_midi::PPQ;
        assert_eq!(
            (clip.fade_in_ticks, clip.fade_out_ticks),
            (beat / 4, beat / 2)
        );
        let channel_db = fl_audio_clip_gain_db(fl.channels[1].settings.volume);
        assert!((clip.gain_db - channel_db - 20.0 * 0.631f64.log10()).abs() < 1e-6);
        assert!((20.0 * 0.631f64.log10() + 4.0).abs() < 0.01);

        // A fade longer than its clip fills the clip and no more.
        let mut long = song();
        if let FlClipContent::AudioSample { fade_out_ms, .. } = &mut long.playlist_clips[1].content
        {
            *fade_out_ms = 10_000.0;
        }
        let built = build_project(&long, "Fades", &here);
        let ClipContent::Audio(clip) = &built.project.track("insert-003").unwrap().clips[0].content
        else {
            panic!("expected audio");
        };
        assert_eq!(clip.fade_out_ticks, long.playlist_clips[1].length_ticks);
    }
}
