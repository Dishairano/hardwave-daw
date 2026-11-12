//! A Hardwave project made from a parsed FL Studio project, in FL's shape.
//!
//! FL shows a song as three lists, and the song comes across as the same
//! three: every channel of FL's channel rack is a channel track in FL's
//! order and filter groups (played or not); every clip sits on the
//! playlist row FL had it on, on the channel it belongs to; and the mixer
//! is the master and the inserts the song uses, numbered and named as in
//! FL. A channel plays into its FL insert, an insert into the master or
//! another insert, as in FL. A channel's volume and pan are the channel's,
//! an automation clip sits on its own automation channel and moves the
//! knob it moved in FL. The import used to put a row's samples on a track
//! per insert they played through, after five hundred empty rows, and
//! folded each channel's level into every clip.
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
use crate::layout::PlaylistRow;
use crate::project::Project;
use crate::track::{Track, TrackKind};
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
            allow_smart_disable: true,
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
    /// Its envelope, loop, voices and the sample's effects, as FL has them;
    /// the app turns these into the built-in sampler's settings.
    pub setup: crate::fl_import::FlSamplerSetup,
}

/// What an audio clip still needs once its file is loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioFixup {
    pub track_id: String,
    pub clip_id: String,
    /// Where the clip starts in its file, in the file's own milliseconds
    /// however the sample is stretched or pitched (for a reversed sample,
    /// counted from the file's end); FL's -1 means "from the start".
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

        // FL keeps a clip's start in the file's own time, whatever the
        // stretch: a loop trimmed two of its bars in starts two of the
        // file's bars in. Read as a place in the stretched sample and
        // divided by the stretch, every trimmed clip of a stretched loop
        // started early or late in it.
        let start = if self.start_ms > 0.0 {
            self.start_ms as f64 / 1000.0 * rate as f64
        } else {
            0.0
        };
        // The clip's length on the timeline, in the file's frames.
        let span = self.length_seconds * rate as f64 / stretch;
        // The engine reads a reversed clip backwards from the end of its
        // window, where FL reverses the whole sample and starts the clip
        // `start` into that: the same sound when the window ends `start`
        // before the end of the file.
        let from = if self.reversed {
            (frames as f64 - start - span).max(0.0)
        } else {
            start
        };
        // Offsets count in the file's own frames; the engine scales them.
        clip.source_start = (from.round() as u64).min(frames);
        clip.source_end = ((from + span).round() as u64).clamp(clip.source_start, frames);
    }
}

/// What came across, and what did not.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FlBuildReport {
    pub bpm: f32,
    /// Channel tracks: one per channel of FL's channel rack.
    pub channels: usize,
    /// Playlist rows: FL's, down to the last one the song uses.
    pub rows: usize,
    pub audio_clips: usize,
    /// MIDI clips made from pattern placements: one per channel the
    /// pattern plays.
    pub pattern_clips: usize,
    /// FL's pattern clips on the playlist. Each comes as a MIDI clip per
    /// channel on its row, not yet as one clip.
    pub pattern_placements: usize,
    pub notes: usize,
    /// Samples the project points at that were not found here.
    pub missing_samples: Vec<String>,
    /// Instrument channels whose plug-in could not come across.
    pub plugins_left_behind: Vec<String>,
    /// Automation clips that came across moving what they moved in FL.
    pub automation_clips: usize,
    /// Automation clips placed on their channel's rows that move nothing
    /// here: what they move has no counterpart.
    pub automation_clips_moving_nothing: usize,
    /// Automation clips whose knob has nothing here to move, by FL's name
    /// for them ("Build Snare - Channel pitch").
    pub automation_left_behind: Vec<String>,
    /// Some automation that came across bends between its points in FL;
    /// here it runs straight.
    pub automation_curves_straightened: bool,
    /// Channels using FL's quick fade knobs on their sample, which do not
    /// come across yet.
    pub sample_fades_left_behind: Vec<String>,
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

/// An audio clip channel's volume as its track's fader. FL plays an audio
/// clip at the file's own level when its channel sits at the 78 % default,
/// so the knob counts from there.
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

/// The playlist row of FL's playlist track `track` (FL counts from 1, the
/// rows from 0).
fn row_of(track: u32) -> u32 {
    track
        .saturating_sub(1)
        .min(crate::sanitize::MAX_ROWS as u32 - 1)
}

/// An FL colour (`#rrggbb`) as an automation clip's ARGB.
fn argb(color: &str) -> Option<u32> {
    let hex = color.strip_prefix('#').filter(|h| h.len() == 6)?;
    u32::from_str_radix(hex, 16)
        .ok()
        .map(|rgb| 0xff00_0000 | rgb)
}

/// What each FL channel became: the track that is the channel, and for a
/// channel playing notes that also has samples on the playlist, the audio
/// track its samples are on.
#[derive(Default)]
struct ChannelTracks {
    track: BTreeMap<u32, String>,
    audio: BTreeMap<u32, String>,
}

/// Make the song. `resolve` turns a sample path as FL wrote it into a
/// file on this machine, or None when it cannot be found.
pub fn build_project(
    fl: &FlProject,
    name: &str,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> FlBuild {
    let mut project = Project::default();
    // FL's shape: the master, a track per channel and one per insert in
    // use, none of the empty rows a new song starts with.
    project
        .tracks
        .retain(|t| matches!(t.kind, TrackKind::Master));
    project.metadata.name = name.to_string();
    // FL's inserts pass a centred sound at 0 dB.
    project.inserts_unity_at_centre = true;
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

    // The rack's filter groups, FL's own. A channel the file puts in no
    // group, or in one it does not name, is listed under "Unsorted".
    let mut groups = fl.rack_groups.clone();
    groups.truncate(crate::sanitize::MAX_RACK_GROUPS - 1);
    let named = groups.len();
    let unsorted = || -> u16 {
        fl.rack_groups
            .iter()
            .take(named)
            .position(|g| g == crate::layout::DEFAULT_RACK_GROUP)
            .unwrap_or(named) as u16
    };
    let group_of = |channel: Option<&FlChannel>| -> u16 {
        channel
            .and_then(|c| c.settings.group)
            .filter(|g| (*g as usize) < named)
            .map_or_else(unsorted, |g| g as u16)
    };
    if fl
        .channels
        .iter()
        .any(|c| group_of(Some(c)) as usize >= named)
        || fl.channels.is_empty()
    {
        groups.push(crate::layout::DEFAULT_RACK_GROUP.to_string());
    }
    project.rack_groups = groups;

    // The arrangement each clip made is in, by the clip's id; those not in
    // the one open in FL are moved into arrangements of their own at the end.
    let mut arrangement_of: BTreeMap<String, usize> = BTreeMap::new();
    // Each track that makes sound, with the FL insert it plays into.
    let mut sources: Vec<(String, u32)> = Vec::new();

    // The channels that play notes, and those with samples on the playlist.
    let with_notes: BTreeSet<u32> = fl
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
    let with_samples: BTreeSet<u32> = fl
        .playlist_clips
        .iter()
        .filter_map(|c| match &c.content {
            FlClipContent::AudioSample { channel_index, .. } => Some(*channel_index),
            _ => None,
        })
        .collect();

    // A track per channel, in the rack's order, played or not.
    let mut made = ChannelTracks::default();
    for (index, channel) in fl.channels.iter().enumerate() {
        let index = index as u32;
        let name = channel_name(index);
        let id = uuid::Uuid::new_v4().to_string();
        let settings = &channel.settings;
        let kind = match channel.kind {
            FlChannelKind::Automation => TrackKind::Automation,
            FlChannelKind::AudioClip => TrackKind::Audio,
            FlChannelKind::Sampler | FlChannelKind::Instrument | FlChannelKind::Layer => {
                TrackKind::Midi
            }
            FlChannelKind::Other if with_notes.contains(&index) => TrackKind::Midi,
            FlChannelKind::Other => TrackKind::Audio,
        };
        let mut track = match kind {
            TrackKind::Automation => Track::new_automation(id.clone(), name.clone()),
            TrackKind::Midi => Track::new_midi(id.clone(), name.clone()),
            _ => Track::new_audio(id.clone(), name.clone()),
        };
        track.rack_group = Some(group_of(Some(channel)));
        if let Some(color) = &channel.color {
            track.color = color.clone();
        }
        match kind {
            TrackKind::Automation => {}
            TrackKind::Midi => {
                track.volume_db = fl_volume_db(settings.volume);
                track.pan = fl_pan(settings.pan);
                track.muted = !settings.enabled;
                sources.push((id.clone(), channel_insert(channel)));
            }
            _ => {
                // An audio clip channel's level, pan, stereo swap and
                // polarity, which FL applies to each of its clips.
                track.volume_db = fl_audio_clip_gain_db(settings.volume);
                track.pan = fl_pan(settings.pan);
                track.swap_lr = settings.swap_stereo;
                track.phase_invert = settings.polarity_inverted;
                track.muted = !settings.enabled;
                sources.push((id.clone(), channel_insert(channel)));
                made.audio.insert(index, id.clone());
            }
        }
        if matches!(kind, TrackKind::Midi) {
            match (channel.kind, &channel.sample_path) {
                // A sampler plays its sample in the built-in sampler, also
                // when no pattern plays it, so it is there to play.
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
                            setup: settings.sampler,
                        });
                    }
                    if with_notes.contains(&index)
                        && (settings.fade_in > 0 || settings.fade_out > 0)
                    {
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
                    track.inserts.insert(0, slot);
                    // Without its settings it would play its default sound
                    // where FL played the song's.
                    track.muted |= !carried;
                }
                // A layer has no sound of its own: its notes play on its
                // children (routed to them below).
                (FlChannelKind::Layer, _) if !channel.layer_children.is_empty() => {
                    track.muted = true;
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
                    track.muted = true;
                }
                _ => {}
            }
        }
        project.tracks.push(track);
        made.track.insert(index, id.clone());

        // A channel playing notes that also has samples on the playlist
        // (FL does not make one, but a file can say so): its samples go on
        // an audio track of their own beside it, where they can play.
        if matches!(kind, TrackKind::Midi) && with_samples.contains(&index) {
            let audio_id = uuid::Uuid::new_v4().to_string();
            let mut audio = Track::new_audio(audio_id.clone(), format!("{name} (audio)"));
            audio.rack_group = Some(group_of(Some(channel)));
            audio.color = project
                .tracks
                .last()
                .map_or(audio.color, |t| t.color.clone());
            audio.volume_db = fl_audio_clip_gain_db(settings.volume);
            audio.pan = fl_pan(settings.pan);
            audio.muted = !settings.enabled;
            sources.push((audio_id.clone(), channel_insert(channel)));
            made.audio.insert(index, audio_id);
            project.tracks.push(audio);
        }
    }
    // Samples of a channel the rack does not have still come, on a channel
    // of their own, so the song keeps its shape.
    for index in &with_samples {
        if made.audio.contains_key(index) {
            continue;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut track = Track::new_audio(id.clone(), channel_name(*index));
        track.rack_group = Some(unsorted());
        if project.rack_groups.len() <= unsorted() as usize {
            project
                .rack_groups
                .push(crate::layout::DEFAULT_RACK_GROUP.to_string());
        }
        sources.push((id.clone(), 0));
        made.audio.insert(*index, id.clone());
        made.track.insert(*index, id);
        project.tracks.push(track);
    }
    report.channels = project
        .tracks
        .iter()
        .filter(|t| t.rack_group.is_some())
        .count();

    // A layer plays its notes on its children.
    for (index, layer_id) in &made.track {
        let children: Vec<String> = fl
            .channels
            .get(*index as usize)
            .map_or(&[][..], |c| &c.layer_children)
            .iter()
            .filter(|child| {
                matches!(
                    fl.channels.get(**child as usize).map(|c| c.kind),
                    Some(FlChannelKind::Sampler | FlChannelKind::Instrument | FlChannelKind::Layer)
                )
            })
            .filter_map(|child| made.track.get(child).cloned())
            .filter(|child| child != layer_id)
            .collect();
        if let (false, Some(track)) = (children.is_empty(), project.track_mut(layer_id)) {
            track.midi_route_to = children;
        }
    }

    // Each placement of a pattern brings its notes, per channel, to that
    // channel's track, on the pattern clip's row.
    let pattern_name = |pattern: u32| -> String {
        fl.pattern_names
            .iter()
            .find(|(id, _)| *id == pattern)
            .map(|(_, name)| name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Pattern {pattern}"))
    };
    for clip in &fl.playlist_clips {
        let FlClipContent::Pattern {
            pattern_index,
            offset_ticks,
        } = &clip.content
        else {
            continue;
        };
        report.pattern_placements += 1;
        for (pattern, channel, notes) in &fl.pattern_notes {
            if pattern != pattern_index || notes.is_empty() || !with_notes.contains(channel) {
                continue;
            }
            let Some(track_id) = made.track.get(channel) else {
                continue;
            };
            let mut midi = hardwave_midi::MidiClip::new(
                uuid::Uuid::new_v4().to_string(),
                pattern_name(*pattern),
                clip.length_ticks.max(1),
            );
            // A start offset past every note is not one FL could have set
            // (a field read as something else): the clip plays from the
            // pattern's start rather than silent.
            let pattern_end = notes
                .iter()
                .map(|n| n.tick + n.length_ticks)
                .max()
                .unwrap_or(0);
            let offset = if *offset_ticks >= pattern_end {
                0
            } else {
                *offset_ticks
            };
            // Notes past the end of the placement are cut off in FL too. A
            // note held over where a clip trimmed at its start begins plays
            // its tail from there, as FL plays truncated notes. A muted
            // placement keeps its notes, silent.
            midi.notes = notes
                .iter()
                .filter(|n| n.tick + n.length_ticks > offset && n.tick < offset + clip.length_ticks)
                .map(|n| hardwave_midi::MidiNote {
                    start_tick: n.tick.saturating_sub(offset),
                    duration_ticks: n.length_ticks - offset.saturating_sub(n.tick),
                    pitch: n.pitch,
                    velocity: n.velocity as f32 / 127.0,
                    channel: 0,
                    muted: clip.muted,
                    ..Default::default()
                })
                .collect();
            report.notes += midi.notes.len();
            let id = uuid::Uuid::new_v4().to_string();
            arrangement_of.insert(id.clone(), clip.arrangement);
            if let Some(track) = project.track_mut(track_id) {
                track.clips.push(ClipPlacement {
                    content: ClipContent::Midi(MidiClipRef { id, clip: midi }),
                    track_id: track_id.clone(),
                    position_ticks: clip.start_tick,
                    length_ticks: clip.length_ticks.max(1),
                    lane: 0,
                    // FL draws a pattern clip in the pattern's colour.
                    color: fl
                        .pattern_colors
                        .iter()
                        .find(|(p, _)| p == pattern_index)
                        .map(|(_, c)| c.clone()),
                    row: Some(row_of(clip.track_index)),
                });
                report.pattern_clips += 1;
            }
        }
    }

    // Every sample on the playlist, on its channel, on its FL row, in FL's
    // order. The channel's level, pan, swap and polarity are on its track;
    // the clip keeps its own volume and fades.
    let mut faded = BTreeSet::new();
    for clip in &fl.playlist_clips {
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
        let Some(track_id) = made.audio.get(channel_index).cloned() else {
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
        // where FL had it, so the song keeps its shape and the file can be
        // found later.
        let file = find(sample_path).unwrap_or_else(|| sample_path.clone());
        let clip_id = uuid::Uuid::new_v4().to_string();
        arrangement_of.insert(clip_id.clone(), clip.arrangement);
        // The clip's own fades, in milliseconds, and its own volume.
        let length = clip.length_ticks.max(1);
        let ticks = |ms: f32| ((ms as f64 / 1000.0 / seconds_per_tick).round() as u64).min(length);
        let fade_in_ticks = ticks(*fade_in_ms);
        let fade_out_ticks = ticks(*fade_out_ms).min(length - fade_in_ticks);
        let own_gain_db = if *volume > 0.0 {
            (20.0 * (*volume as f64).log10()).max(-100.0)
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
            gain_db: own_gain_db,
            fade_in_ticks,
            fade_out_ticks,
            muted: clip.muted,
            reversed: settings.reversed,
            pitch_semitones: 0.0,
            stretch_ratio: 1.0,
            fade_in_curve: FadeCurve::default(),
            fade_out_curve: FadeCurve::default(),
            warp_markers: Vec::new(),
            source_file: file,
            // The sample's own effects, as FL keeps them on its channel:
            // they reach every clip of it.
            treatment: {
                let fx = &settings.sampler;
                SampleTreatment {
                    normalize: fx.normalize,
                    remove_dc: fx.remove_dc,
                    fade_stereo: fx.fade_stereo,
                    trim: fx.trim.clamp(0.0, 1.0) as f64,
                    declick: crate::clip::DeclickMode::from_fl(fx.declick).unwrap_or_default(),
                    start: fx.start.unwrap_or(0.0).clamp(0.0, 1.0) as f64,
                    length: fx.length.unwrap_or(1.0).clamp(0.0, 1.0) as f64,
                    ..Default::default()
                }
            },
            tuned_notes: Vec::new(),
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
                length_ticks: length,
                lane: 0,
                // FL draws an audio clip in its channel's colour.
                color: channel.and_then(|c| c.color.clone()),
                row: Some(row_of(clip.track_index)),
            });
            report.audio_clips += 1;
        }
    }

    let mut mixer = build_mixer(
        fl,
        &mut project,
        &sources,
        &mut external_plugins,
        &mut report,
    );
    let places = AutomationPlaces {
        channels: &made,
        external: &external_plugins,
    };
    build_automation(
        fl,
        &mut project,
        &places,
        &mut mixer,
        &mut report,
        &mut arrangement_of,
    );
    project.rows = rows_of(fl, fl.current_arrangement);
    report.rows = project.rows.len();
    split_arrangements(&mut project, fl, &arrangement_of);

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

/// How many playlist rows the song needs: FL's, down to the last one a
/// clip sits on or FL named or switched, in any arrangement.
fn row_count(fl: &FlProject) -> usize {
    let tracks = fl
        .playlist_clips
        .iter()
        .map(|c| c.track_index)
        .chain(fl.playlist_track_names.iter().map(|(t, _)| *t))
        .chain(fl.playlist_tracks_off.iter().copied())
        .chain(fl.playlist_tracks_solo.iter().copied())
        .chain(fl.arrangements.iter().flat_map(|a| {
            a.track_names
                .iter()
                .map(|(t, _)| *t)
                .chain(a.tracks_off.iter().copied())
                .chain(a.tracks_solo.iter().copied())
        }));
    tracks.map(|t| row_of(t) as usize + 1).max().unwrap_or(0)
}

/// FL's playlist tracks in `arrangement` as our rows: their names, colours
/// and switches. FL soloes a track by switching every other one off, so
/// while one is soloed the switches say nothing else: the solo comes
/// across as a solo, which gives everything back with one click.
fn rows_of(fl: &FlProject, arrangement: usize) -> Vec<PlaylistRow> {
    // A file from before FL's arrangements keeps them for its one playlist.
    let (names, colors, off, solo) = match fl.arrangements.get(arrangement) {
        Some(a) => (
            &a.track_names,
            &a.track_colors,
            &a.tracks_off,
            &a.tracks_solo,
        ),
        None => (
            &fl.playlist_track_names,
            &fl.playlist_track_colors,
            &fl.playlist_tracks_off,
            &fl.playlist_tracks_solo,
        ),
    };
    let find = |list: &[(u32, String)], track: u32| {
        list.iter()
            .find(|(t, _)| *t == track)
            .map(|(_, v)| v.clone())
    };
    (1..=row_count(fl) as u32)
        .map(|track| PlaylistRow {
            name: find(names, track).unwrap_or_default(),
            color: find(colors, track),
            muted: solo.is_empty() && off.contains(&track),
            soloed: solo.contains(&track),
        })
        .collect()
}

/// FL's arrangements as ours: the open one stays on the tracks, each other
/// one is stored with its clips and its own rows, by name and in FL's
/// order. Every FL arrangement's clips used to land on the one playlist at
/// once.
fn split_arrangements(
    project: &mut Project,
    fl: &FlProject,
    arrangement_of: &BTreeMap<String, usize>,
) {
    if fl.arrangements.len() < 2 {
        return;
    }
    let open = fl.current_arrangement;
    let id_of = |content: &ClipContent| -> String {
        match content {
            ClipContent::Midi(m) => m.id.clone(),
            ClipContent::Audio(a) => a.id.clone(),
        }
    };
    let elsewhere =
        |id: String| -> Option<usize> { arrangement_of.get(&id).copied().filter(|a| *a != open) };
    let mut stored: Vec<std::collections::BTreeMap<String, crate::arrangement::TrackTimeline>> =
        vec![Default::default(); fl.arrangements.len()];
    for track in &mut project.tracks {
        let mut kept = Vec::new();
        for placement in std::mem::take(&mut track.clips) {
            match elsewhere(id_of(&placement.content)) {
                Some(a) => stored[a]
                    .entry(track.id.clone())
                    .or_default()
                    .clips
                    .push(placement),
                None => kept.push(placement),
            }
        }
        track.clips = kept;
        let mut kept = Vec::new();
        for automation in std::mem::take(&mut track.automation_clips) {
            match elsewhere(automation.id.clone()) {
                Some(a) => stored[a]
                    .entry(track.id.clone())
                    .or_default()
                    .automation_clips
                    .push(automation),
                None => kept.push(automation),
            }
        }
        track.automation_clips = kept;
    }
    let mut arrangements = Vec::new();
    for (index, (fl_arrangement, timelines)) in fl.arrangements.iter().zip(stored).enumerate() {
        arrangements.push(crate::arrangement::Arrangement {
            id: uuid::Uuid::new_v4().to_string(),
            name: fl_arrangement.name.clone(),
            timelines,
            rows: rows_of(fl, index),
        });
        if index == open {
            project.active_arrangement = arrangements[index].id.clone();
        }
    }
    project.arrangements = arrangements;
    // The open one's snapshot is what is on the tracks now.
    project.capture_active_arrangement();
}

/// Where the channels went, for their automation to find them.
struct AutomationPlaces<'a> {
    channels: &'a ChannelTracks,
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
    /// An audio clip channel's volume as its track's fader, counted from
    /// FL's 78 % default as the track's level is.
    AudioChannel,
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
            Scale::AudioChannel => fader(fl_audio_clip_gain_db((v * 12_800.0) as u32)),
            Scale::EqGain => (((v * 2.0 - 1.0) * 18.0 + 24.0) / 48.0).clamp(0.0, 1.0),
            Scale::EqFreq => ((20.0 * 1000f64.powf(v) - 20.0) / 19_980.0).clamp(0.0, 1.0),
        }
    }
}

/// FL's automation clips, on their own automation channels, each moving
/// the knob it moved in FL on the track that has it. A clip moving
/// something that has no counterpart here still sits where FL had it,
/// moving nothing, and is named in the report. The points come across
/// where they are; FL's curves between them run straight here.
fn build_automation(
    fl: &FlProject,
    project: &mut Project,
    places: &AutomationPlaces,
    mixer: &mut MixerMap,
    report: &mut FlBuildReport,
    arrangement_of: &mut BTreeMap<String, usize>,
) {
    use crate::automation::{AutomationTarget, CurveMode};
    use crate::fl_import::FlAutomationTarget as T;

    let kind_of = |project: &Project, id: &str| project.track(id).map(|t| t.kind.clone());
    // Each automation channel's knob, as (track, target, scale) here.
    let mut resolved: BTreeMap<u32, Vec<(String, AutomationTarget, Scale)>> = BTreeMap::new();
    for link in &fl.automation_links {
        let mut found = Vec::new();
        let instrument_plugin = |channel: u32| {
            let track = places.channels.track.get(&channel)?;
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
                // The channel's own track, and the audio track beside it
                // where it has one.
                let tracks = places
                    .channels
                    .track
                    .get(&channel)
                    .into_iter()
                    .chain(places.channels.audio.get(&channel))
                    .collect::<BTreeSet<_>>();
                for track in tracks {
                    let scale = match kind_of(project, track) {
                        Some(TrackKind::Midi) => Scale::Fader {
                            top: 12_800.0,
                            offset_db: 0.0,
                        },
                        Some(TrackKind::Audio) => Scale::AudioChannel,
                        _ => continue,
                    };
                    found.push(if volume {
                        (track.clone(), AutomationTarget::TrackVolume, scale)
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
        // Muted on the playlist: it moves nothing in FL either, and an
        // automation clip here has no mute of its own.
        if clip.muted {
            continue;
        }
        let targets = resolved
            .get(channel_index)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let channel = fl.channels.get(*channel_index as usize);
        let points = channel.map(|c| c.automation.as_slice()).unwrap_or(&[]);
        // Its own automation channel, where FL lists it.
        let home = places
            .channels
            .track
            .get(channel_index)
            .filter(|id| kind_of(project, id) == Some(TrackKind::Automation))
            .cloned();
        let moves = !targets.is_empty() && !points.is_empty();
        if !moves && left_behind.insert(name.clone()) {
            report.automation_left_behind.push(name.clone());
        }
        let length = clip.length_ticks.max(1);
        let window = automation_window(points, *offset_ticks, length);
        let color = channel.and_then(|c| c.color.as_deref()).and_then(argb);
        let mut place = |project: &mut Project,
                         on: &str,
                         target: AutomationTarget,
                         moves_track: Option<String>,
                         scale: Scale| {
            let mut automation = crate::automation_clip::AutomationClip::new(
                uuid::Uuid::new_v4().to_string(),
                target,
                clip.start_tick,
                length,
            );
            for (tick, value) in &window {
                automation.insert_point(*tick, scale.apply(*value), CurveMode::Linear);
            }
            automation.row = Some(row_of(clip.track_index));
            automation.target_track = moves_track;
            if let Some(color) = color {
                automation.color_argb = color;
            }
            arrangement_of.insert(automation.id.clone(), clip.arrangement);
            match project.track_mut(on) {
                Some(track) => {
                    track.automation_clips.push(automation);
                    true
                }
                None => false,
            }
        };
        if !moves {
            // It sits where FL had it with FL's points, on its channel,
            // moving nothing: an automation channel plays nothing itself.
            if let Some(home) = &home {
                if place(
                    project,
                    home,
                    AutomationTarget::TrackVolume,
                    None,
                    Scale::Same,
                ) {
                    report.automation_clips_moving_nothing += 1;
                }
            }
            continue;
        }
        report.automation_curves_straightened |= points.iter().any(|p| p.tension.abs() > 0.01);
        for (track_id, target, scale) in targets {
            // On its channel, moving the track's knob; a channel that is not
            // an automation channel puts it on the track it moves.
            let placed = match &home {
                Some(home) => place(
                    project,
                    home,
                    target.clone(),
                    Some(track_id.clone()),
                    *scale,
                ),
                None => place(project, track_id, target.clone(), None, *scale),
            };
            if placed {
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

/// FL's mixer, numbered as in FL. An insert comes as a track when it holds
/// something of the song: a name, a colour, an effect, a fader away from
/// FL's 0 dB, or sound routed into it (from a channel, or from another
/// insert that comes). FL's "current" insert never does. Each track made
/// so far plays into the insert its channel was routed to; the inserts
/// keep FL's fader, pan and switches, and send on where FL sent them.
/// Insert 0 is the master.
fn build_mixer(
    fl: &FlProject,
    project: &mut Project,
    sources: &[(String, u32)],
    external: &mut Vec<ExternalPlugin>,
    report: &mut FlBuildReport,
) -> MixerMap {
    let mut map = MixerMap::default();
    let inserts: BTreeMap<u32, &FlMixerTrack> = fl
        .mixer
        .iter()
        .filter(|m| !m.current && m.index <= crate::sanitize::MAX_MIXER_NUMBER as u32)
        .map(|m| (m.index, m))
        .collect();
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
    let holds_something = |m: &FlMixerTrack| {
        !m.name.is_empty() || m.color.is_some() || !m.slots.is_empty() || m.volume != 12_800
    };
    let mut used = BTreeSet::new();
    let mut queue: Vec<u32> = sources
        .iter()
        .map(|(_, insert)| *insert)
        .chain(
            inserts
                .values()
                .filter(|m| holds_something(m))
                .map(|m| m.index),
        )
        .collect();
    while let Some(insert) = queue.pop() {
        let Some(m) = inserts.get(&insert) else {
            continue;
        };
        if insert != 0 && used.insert(insert) {
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
    for insert in &used {
        let m = inserts[insert];
        let id = uuid::Uuid::new_v4().to_string();
        let mut track = Track::new_bus(id.clone(), name_of(m));
        track.mixer_number = Some(*insert as u16);
        if let Some(color) = &m.color {
            track.color = color.clone();
        }
        project.tracks.push(track);
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
    let mut sent_to = BTreeSet::new();
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
        }
        // The route that carries it on, to master or to another insert, at
        // its level. A route to master below full used to play at full.
        let onward = main.or_else(|| routes.iter().find(|r| r.to == 0).copied());
        // The route on comes after the fader: the same as a fader that much
        // lower, with the other routes (sends here, after the fader too)
        // raised by as much so they stay where FL had them. Only the route
        // with no others was taken; a group at -6 dB with a reverb send
        // played at full.
        let mut offset_db = 0.0;
        if let Some(main) = onward {
            if main.level != 12_800 {
                let offset = fl_volume_db(main.level as u32);
                if offset > -60.0 || routes.len() == 1 {
                    offset_db = offset;
                    track.volume_db = (track.volume_db + offset).max(-100.0);
                    map.fader_offset_db.insert(*insert, offset);
                } else {
                    // Silent on to master but still sending: no fader can
                    // say that.
                    report.route_levels_left_behind.push(name.clone());
                }
            }
        }
        track.sends = routes
            .iter()
            .filter(|r| r.to != 0 && Some(**r) != main)
            .filter_map(|r| {
                sent_to.insert(r.to);
                Some(crate::mixer::Send {
                    target: track_of(r.to)?,
                    gain_db: (fl_volume_db(r.level as u32) - offset_db).min(24.0),
                    pre_fader: false,
                    enabled: true,
                })
            })
            .collect();
        track.inserts = effects(m, &name, track_id, &mut map, external, report);
    }
    // An insert nothing reaches stays a group, as every insert is in FL.
    for (insert, track_id) in &insert_track {
        if let Some(track) = project.track_mut(track_id) {
            track.kind = if !fed.contains(insert) && sent_to.contains(insert) {
                TrackKind::Return
            } else {
                TrackKind::Bus
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
            allow_smart_disable: true,
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
    use crate::fl_import::{FlArrangement, FlChannel, FlNote, FlPlaylistClip};

    fn channel(
        name: &str,
        kind: FlChannelKind,
        sample: Option<&str>,
        plugin: Option<&str>,
    ) -> FlChannel {
        FlChannel {
            color: None,
            layer_children: Vec::new(),
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
                    arrangement: 0,
                    track_index: 1,
                    start_tick: 0,
                    length_ticks: 3840,
                    content: FlClipContent::Pattern {
                        pattern_index: 1,
                        offset_ticks: 0,
                    },
                    muted: false,
                },
                FlPlaylistClip {
                    arrangement: 0,
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
                    arrangement: 0,
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
                    arrangement: 0,
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
                    arrangement: 0,
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
            arrangements: Vec::new(),
            current_arrangement: 0,
            pattern_colors: Vec::new(),
            playlist_track_colors: Vec::new(),
            rack_groups: Vec::new(),
        }
    }

    fn here(path: &str) -> Option<String> {
        path.starts_with("C:\\packs")
            .then(|| path.replace("C:\\packs", "/home/me/packs"))
    }

    /// The channel track named `name`.
    fn channel_track<'a>(p: &'a Project, name: &str) -> &'a crate::track::Track {
        p.tracks
            .iter()
            .find(|t| t.name == name && t.rack_group.is_some())
            .unwrap_or_else(|| panic!("no channel {name}"))
    }

    /// The track that is insert `number` in the mixer.
    fn insert_track(p: &Project, number: u16) -> &crate::track::Track {
        p.tracks
            .iter()
            .find(|t| t.mixer_number == Some(number))
            .unwrap_or_else(|| panic!("no insert {number}"))
    }

    /// FL's three lists: every channel of the rack a track, in the rack's
    /// order, played or not; no empty rows; no insert nothing uses.
    #[test]
    fn every_channel_is_a_track_in_the_rack_s_order() {
        let built = build_project(&song(), "Industrial", &here);
        let p = &built.project;
        let names: Vec<&str> = p.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Master",
                "Screech",
                "Kick render",
                "808 Kick",
                "Cutoff",
                "Lost vocal"
            ],
            "the master and one track per channel; none of a new song's 500"
        );
        let kinds: Vec<TrackKind> = p.tracks[1..].iter().map(|t| t.kind.clone()).collect();
        assert_eq!(
            kinds,
            [
                TrackKind::Midi,
                TrackKind::Audio,
                TrackKind::Midi,
                TrackKind::Automation,
                TrackKind::Audio
            ]
        );
        assert!(p.tracks[1..]
            .iter()
            .all(|t| t.rack_group == Some(0) && t.mixer_number.is_none()));
        assert_eq!(p.rack_groups, ["Unsorted"], "FL named no groups");
        assert_eq!(built.report.channels, 5);
        assert_eq!(
            built.report.mixer_inserts, 0,
            "everything plays into the master"
        );
        assert!(p.tracks[1..].iter().all(|t| t.output_bus.is_none()));
        assert_eq!(p.metadata.name, "Industrial");
        assert_eq!(p.tempo_map.entries[0].bpm, 160.0);
    }

    /// FL's rows down to the last one used, with their names, and every
    /// clip on its FL row whatever channel it is on.
    #[test]
    fn clips_sit_on_fl_s_rows() {
        let built = build_project(&song(), "Industrial", &here);
        let p = &built.project;
        assert_eq!(p.rows.len(), 9, "FL's track 9 is the last one used");
        assert_eq!(built.report.rows, 9);
        assert_eq!(p.rows[8].name, "Vocals", "a named FL track keeps its name");
        assert_eq!(p.rows[0].name, "", "the others read Track N");
        let rows = |name: &str| -> Vec<Option<u32>> {
            let t = channel_track(p, name);
            t.clips
                .iter()
                .map(|c| c.row)
                .chain(t.automation_clips.iter().map(|c| c.row))
                .collect()
        };
        assert_eq!(rows("Screech"), [Some(0)]);
        assert_eq!(rows("Kick render"), [Some(4), Some(4)]);
        assert_eq!(rows("Lost vocal"), [Some(8)]);
        assert_eq!(rows("Cutoff"), [Some(1)]);
        // Opened as a song, it keeps all of it.
        let opened = Project::from_bytes(&p.to_bytes().unwrap()).unwrap();
        assert_eq!(opened.rows, p.rows);
        let fields = |p: &Project| -> Vec<_> {
            p.tracks
                .iter()
                .map(|t| (t.id.clone(), t.mixer_number, t.rack_group))
                .collect()
        };
        assert_eq!(fields(&opened), fields(p));
    }

    #[test]
    fn patterns_land_on_their_channels_and_samples_where_fl_had_them() {
        let built = build_project(&song(), "Industrial", &here);
        let p = &built.project;
        let screech = channel_track(p, "Screech");
        assert_eq!(screech.clips.len(), 1);
        match &screech.clips[0].content {
            ClipContent::Midi(m) => {
                assert_eq!(m.clip.notes.len(), 2);
                assert_eq!(m.clip.name, "Drop");
            }
            _ => panic!("expected notes"),
        }
        let kicks = channel_track(p, "Kick render");
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
            channel_track(&built.project, "Screech").muted,
            "the track whose plug-in did not come is muted"
        );
        assert!(
            !channel_track(&built.project, "808 Kick").muted,
            "the sampler track plays"
        );
        assert_eq!(built.audio_fixups.len(), 3, "one for every audio clip");
        assert_eq!(built.audio_fixups[0].start_ms, 20.0);
        assert_eq!(
            built.sampler_samples,
            vec![SamplerSample {
                track_id: channel_track(&built.project, "808 Kick").id.clone(),
                file: "/home/me/packs\\808.wav".to_string(),
                root_note: 60,
                reversed: false,
                swap_stereo: false,
                polarity_inverted: false,
                setup: Default::default(),
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
        // The automation clip moves nothing FL told us about: it sits on
        // its channel, on its row, moving nothing.
        assert_eq!(built.report.automation_clips, 0);
        assert_eq!(built.report.automation_clips_moving_nothing, 1);
        assert_eq!(
            built.report.automation_left_behind,
            vec!["Cutoff".to_string()]
        );
        let cutoff = channel_track(&built.project, "Cutoff");
        assert_eq!(cutoff.automation_clips.len(), 1);
        assert_eq!(cutoff.automation_clips[0].target_track, None);
        // The missing sample still has its clip, pointing where FL had it.
        let vocals = channel_track(&built.project, "Lost vocal");
        assert_eq!(vocals.clips.len(), 1);
    }

    /// Channels no pattern plays come too, ready to play: a sampler with
    /// its sample in the built-in sampler, an instrument whose plug-in
    /// cannot come muted. Each is in its FL group.
    #[test]
    fn channels_that_never_play_come_ready_and_in_fl_s_groups() {
        let mut fl = song();
        fl.pattern_notes.clear();
        fl.playlist_clips
            .retain(|c| !matches!(c.content, FlClipContent::Pattern { .. }));
        fl.rack_groups = vec!["Audio".into(), "Automation".into(), "Unsorted".into()];
        fl.channels.push(channel(
            "FLEX Bass",
            FlChannelKind::Instrument,
            None,
            Some("FLEX"),
        ));
        for (c, g) in fl.channels.iter_mut().zip([2, 0, 2, 1, 0, 9]) {
            c.settings.group = Some(g);
        }
        let built = build_project(&fl, "Kit", &here);
        let p = &built.project;
        assert_eq!(p.rack_groups, ["Audio", "Automation", "Unsorted"]);
        let group = |name: &str| channel_track(p, name).rack_group;
        assert_eq!(group("808 Kick"), Some(2));
        assert_eq!(group("Kick render"), Some(0));
        assert_eq!(group("Cutoff"), Some(1));
        assert_eq!(
            group("FLEX Bass"),
            Some(2),
            "a group FL does not name is Unsorted"
        );
        let kick = channel_track(p, "808 Kick");
        assert!(kick.clips.is_empty() && !kick.muted);
        assert_eq!(built.sampler_samples.len(), 1);
        assert_eq!(built.sampler_samples[0].track_id, kick.id);
        let flex = channel_track(p, "FLEX Bass");
        assert!(flex.muted && flex.inserts.is_empty());
        assert!(matches!(flex.kind, TrackKind::Midi));
        assert_eq!(
            built.report.plugins_left_behind,
            ["Screech".to_string(), "FLEX Bass (FLEX)".into()]
        );
        assert_eq!(built.report.pattern_placements, 0);
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

    /// A channel's level, pan and switch are on its track, as FL keeps them
    /// on the channel; they used to be folded into every clip.
    #[test]
    fn a_channel_s_settings_are_on_its_track() {
        let built = build_project(&lane_song(), "Industrial", &here);
        let p = &built.project;
        let kicks = channel_track(p, "Kick render");
        assert_eq!(kicks.pan, 0.5, "the channel's pan");
        assert!((kicks.volume_db - fl_audio_clip_gain_db(7_456)).abs() < 1e-9);
        let audio = |i: usize| match &kicks.clips[i].content {
            ClipContent::Audio(a) => a.clone(),
            _ => panic!("expected audio"),
        };
        assert_eq!(audio(0).gain_db, 0.0, "the clip keeps only its own level");
        assert_eq!(audio(0).treatment.pan, 0.0, "and pans with its channel");
        assert!(audio(0).reversed);
        assert!(!audio(0).muted);
        assert!(audio(1).muted, "muted on the playlist");
        let vocals = channel_track(p, "Lost vocal");
        assert!(vocals.muted, "its channel is off");
        match &vocals.clips[0].content {
            ClipContent::Audio(a) => assert!(!a.muted, "the clip itself is not"),
            _ => panic!("expected audio"),
        }
        assert!(p.rows[8].muted, "its FL track is switched off: the row is");
        assert!(!p.rows[4].muted);
        let sampler = channel_track(p, "808 Kick");
        assert_eq!(sampler.volume_db, 0.0);
        assert_eq!(
            built.sampler_samples[0].root_note, 62,
            "two semitones down is the root two up"
        );
        let screech = channel_track(p, "Screech");
        assert_eq!(format!("{:.1}", screech.volume_db), "-5.2");
    }

    #[test]
    fn a_sample_s_own_effects_reach_its_audio_clips() {
        let mut fl = lane_song();
        for c in fl.channels.iter_mut() {
            let fx = &mut c.settings.sampler;
            fx.normalize = true;
            fx.remove_dc = true;
            fx.fade_stereo = true;
            fx.trim = 0.5;
            fx.declick = 4;
            fx.length = Some(1.0);
        }
        let built = build_project(&fl, "Industrial", &here);
        let kicks = channel_track(&built.project, "Kick render");
        let ClipContent::Audio(a) = &kicks.clips[0].content else {
            panic!("expected audio");
        };
        let t = a.treatment;
        assert!(t.normalize && t.remove_dc && t.fade_stereo);
        assert_eq!(t.trim, 0.5);
        assert_eq!(t.declick, crate::clip::DeclickMode::Smooth);
        assert_eq!((t.start, t.length), (0.0, 1.0));
        // Left alone, a clip keeps FL's defaults: nothing precomputed.
        let plain = build_project(&lane_song(), "Industrial", &here);
        let ClipContent::Audio(b) = &channel_track(&plain.project, "Kick render").clips[0].content
        else {
            panic!("expected audio");
        };
        assert!(!b.treatment.normalize && b.treatment.trim == 0.0);
        assert_eq!(b.treatment.declick, crate::clip::DeclickMode::OutOnly);
    }

    /// Two channels' samples on one row: each stays on its channel, with
    /// the channel's own pan and polarity, on the row FL had it on.
    #[test]
    fn samples_of_two_channels_on_one_row_keep_their_own_channel() {
        let mut fl = lane_song();
        fl.channels[4].settings.pan = 0;
        fl.channels[4].settings.polarity_inverted = true;
        fl.playlist_clips[3].track_index = 5;
        let built = build_project(&fl, "Industrial", &here);
        let p = &built.project;
        let kicks = channel_track(p, "Kick render");
        let vocal = channel_track(p, "Lost vocal");
        assert_eq!((kicks.pan, kicks.phase_invert), (0.5, false));
        assert_eq!((vocal.pan, vocal.phase_invert), (-1.0, true));
        assert!(kicks.clips.iter().all(|c| c.row == Some(4)));
        assert_eq!(vocal.clips[0].row, Some(4));
    }

    /// Samples on one row playing into different inserts each play through
    /// their own channel's insert. The row used to be split into a track
    /// per insert.
    #[test]
    fn samples_on_one_row_play_through_their_own_channel_s_insert() {
        let mut fl = mixer_song();
        fl.playlist_clips[3].track_index = 5;
        let built = build_project(&fl, "Industrial", &here);
        let p = &built.project;
        assert_eq!(
            channel_track(p, "Kick render").output_bus.as_ref(),
            Some(&insert_track(p, 2).id)
        );
        assert_eq!(
            channel_track(p, "Lost vocal").output_bus.as_ref(),
            Some(&insert_track(p, 7).id)
        );
        assert_eq!(insert_track(p, 7).name, "Ghost");
        assert!(!p.tracks.iter().any(|t| t.name.contains("(Insert 2)")));
    }

    #[test]
    fn a_switched_off_track_is_a_switched_off_row() {
        let mut fl = song();
        fl.playlist_tracks_off = vec![1];
        let built = build_project(&fl, "Industrial", &here);
        let p = &built.project;
        assert!(p.rows[0].muted, "FL's track switch is the row's");
        assert!(p.rows[1..].iter().all(|r| !r.muted));
        let screech = channel_track(p, "Screech");
        match &screech.clips[0].content {
            ClipContent::Midi(m) => assert!(
                m.clip.notes.iter().all(|n| !n.muted),
                "the notes are kept as they are; the row silences them"
            ),
            _ => panic!("expected notes"),
        }
        // A muted placement keeps its notes, silent.
        let mut fl = song();
        fl.playlist_clips[0].muted = true;
        let built = build_project(&fl, "Industrial", &here);
        match &channel_track(&built.project, "Screech").clips[0].content {
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
            tuned_notes: Vec::new(),
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
        // FL's offset is in the file's own time: one second in is one second
        // into the file, however long the sample is stretched to last. At
        // twice its length a second of timeline is half a second of file.
        let mut clip = blank_clip();
        AudioFixup {
            start_ms: 1_000.0,
            fit_seconds: Some(4.0),
            ..fixup()
        }
        .apply(&mut clip, 96_000, 48_000);
        assert_eq!(clip.source_start, 48_000);
        assert_eq!(clip.source_end, 72_000, "one second of timeline later");
    }

    /// From a real FL 26 project at 130 BPM: a 126 BPM loop of 7.62 s
    /// (336,000 frames at 44.1 kHz) set to last 16 beats (7.38 s), placed
    /// for one bar from half way through. FL wrote the clip's start as
    /// 3809.52 ms and its end as 5714.27 ms: half the file and three
    /// quarters of it, in the file's own time. Read as time in the
    /// stretched sample, the clip started 121 ms (about a sixteenth) late.
    #[test]
    fn a_trimmed_clip_of_a_stretched_loop_plays_the_part_fl_plays() {
        let beat = 60.0 / 130.0;
        let mut clip = blank_clip();
        AudioFixup {
            start_ms: 3_809.523_7,
            length_seconds: 4.0 * beat,
            fit_seconds: Some(16.0 * beat),
            ..fixup()
        }
        .apply(&mut clip, 336_000, 44_100);
        assert_eq!(clip.source_start, 168_000, "half way into the file");
        let fl_end = (5_714.274 / 1000.0 * 44_100.0) as i64;
        assert!(
            (clip.source_end as i64 - fl_end).abs() <= 1,
            "ends where FL says: {} against {fl_end}",
            clip.source_end
        );
    }

    #[test]
    fn a_tape_s_start_counts_in_the_file_too() {
        // Up an octave in Resample the sample plays twice as fast: a quarter
        // second into the file is still a quarter second into the file, and
        // a second of timeline takes two of the file.
        let mut clip = blank_clip();
        AudioFixup {
            start_ms: 250.0,
            resample: true,
            pitch_semitones: 12.0,
            ..fixup()
        }
        .apply(&mut clip, 96_000, 48_000);
        assert_eq!(clip.source_start, 12_000);
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

    /// A layer plays its notes on its children, which get tracks even with
    /// no notes of their own; the layer itself makes no sound.
    #[test]
    fn a_layer_plays_its_notes_on_its_children() {
        let mut fl = song();
        let screech = fl
            .channels
            .iter()
            .position(|c| c.name == "Screech")
            .expect("screech") as u32;
        fl.channels
            .push(channel("Pad", FlChannelKind::Instrument, None, None));
        let pad = fl.channels.len() as u32 - 1;
        let mut stack = channel("Stack", FlChannelKind::Layer, None, None);
        stack.layer_children = vec![screech, pad];
        fl.channels.push(stack);
        let layer = fl.channels.len() as u32 - 1;
        let (pattern, _, notes) = fl.pattern_notes[0].clone();
        fl.pattern_notes.push((pattern, layer, notes));

        let built = build_project(&fl, "s", &here);
        let p = &built.project;
        let track = |name: &str| {
            p.tracks
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("{name}"))
        };
        let stack = track("Stack");
        assert!(stack.muted, "no sound of its own");
        let mut routed = stack.midi_route_to.clone();
        routed.sort();
        let mut children = vec![track("Screech").id.clone(), track("Pad").id.clone()];
        children.sort();
        assert_eq!(routed, children);
        assert!(
            !stack.clips.is_empty(),
            "its notes are on it, for its children to play"
        );
        assert!(
            !built
                .report
                .plugins_left_behind
                .iter()
                .any(|n| n.starts_with("Stack")),
            "a layer is not a plug-in left behind"
        );
    }

    #[test]
    fn colours_come_across_to_tracks_and_clips() {
        let mut fl = song();
        fl.channels[0].color = Some("#e5243b".into());
        let pattern = fl
            .playlist_clips
            .iter()
            .find_map(|c| match c.content {
                FlClipContent::Pattern { pattern_index, .. } => Some(pattern_index),
                _ => None,
            })
            .expect("the song places a pattern");
        fl.pattern_colors = vec![(pattern, "#22c55e".into())];
        let p = build_project(&fl, "s", &|p| Some(p.to_string())).project;
        let name = fl.channels[0].name.clone();
        let first = p.tracks.iter().find(|t| t.name == name).expect("its track");
        assert_eq!(first.color, "#e5243b");
        let pattern_clips: Vec<Option<&str>> = p
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter(|c| matches!(c.content, ClipContent::Midi(_)))
            .map(|c| c.color.as_deref())
            .collect();
        assert!(!pattern_clips.is_empty());
        assert!(
            pattern_clips.iter().all(|c| *c == Some("#22c55e")),
            "{pattern_clips:?}"
        );
    }

    /// Two arrangements: the one open in FL is on the tracks, the other is
    /// stored as an arrangement of its own, each with only its own clips.
    #[test]
    fn fl_arrangements_become_arrangements() {
        let mut fl = song();
        let total = fl.playlist_clips.len();
        assert!(total >= 2);
        // The first clip is in arrangement 0, every other in arrangement 1,
        // which FL had open.
        for (i, clip) in fl.playlist_clips.iter_mut().enumerate() {
            clip.arrangement = usize::from(i > 0);
        }
        fl.arrangements = vec![
            FlArrangement {
                name: "Radio edit".into(),
                ..Default::default()
            },
            FlArrangement {
                name: "Extended".into(),
                ..Default::default()
            },
        ];
        fl.current_arrangement = 1;
        let all = build_project(&song(), "s", &|p| Some(p.to_string())).project;
        let split = build_project(&fl, "s", &|p| Some(p.to_string())).project;

        let count = |p: &Project| -> usize {
            p.tracks
                .iter()
                .map(|t| t.clips.len() + t.automation_clips.len())
                .sum()
        };
        let names: Vec<&str> = split.arrangements.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Radio edit", "Extended"]);
        assert_eq!(split.active_arrangement, split.arrangements[1].id);
        let stored: usize = split.arrangements[0]
            .timelines
            .values()
            .map(|t| t.clips.len() + t.automation_clips.len())
            .sum();
        assert!(stored > 0, "the other arrangement keeps its clips");
        assert_eq!(
            count(&split) + stored,
            count(&all),
            "nothing lost, nothing doubled"
        );
        // Switching to it puts its clips on the tracks.
        let mut switched = split.clone();
        let other = switched.arrangements[0].id.clone();
        switched.capture_active_arrangement();
        switched.apply_arrangement(&other).expect("it is there");
        assert_eq!(count(&switched), stored);
    }

    /// A row switched off in one arrangement and on in another: each
    /// arrangement's clips follow their own switch. On the track, the open
    /// arrangement's switch silenced the row in every arrangement.
    #[test]
    fn a_row_switched_off_in_one_arrangement_plays_in_the_other() {
        let mut fl = song();
        for clip in fl.playlist_clips.iter_mut() {
            clip.arrangement = 1;
        }
        // The second kick clip is in the other arrangement.
        fl.playlist_clips[2].arrangement = 0;
        fl.arrangements = vec![
            FlArrangement {
                name: "Extended".into(),
                ..Default::default()
            },
            FlArrangement {
                name: "Radio edit".into(),
                tracks_off: vec![5],
                ..Default::default()
            },
        ];
        fl.current_arrangement = 1;
        fl.arrangements[1].track_names = vec![(5, "Kicks".into())];
        fl.arrangements[1].track_colors = vec![(5, "#e5243b".into())];
        let p = build_project(&fl, "s", &|p| Some(p.to_string())).project;
        let kicks = channel_track(&p, "Kick render");
        assert!(
            !kicks.muted,
            "the channel is not muted in every arrangement"
        );
        let ClipContent::Audio(open) = &kicks.clips[0].content else {
            panic!("expected audio");
        };
        assert!(!open.muted, "the row's switch is the row's, not the clip's");
        assert!(p.rows[4].muted, "switched off where it is");
        assert_eq!(p.rows[4].name, "Kicks");
        assert_eq!(p.rows[4].color.as_deref(), Some("#e5243b"));
        let other = p
            .arrangements
            .iter()
            .find(|a| a.name == "Extended")
            .unwrap();
        assert!(!other.rows[4].muted, "on in the other arrangement");
        assert_eq!(other.rows[4].name, "", "with its own names");
        let open = p
            .arrangements
            .iter()
            .find(|a| a.name == "Radio edit")
            .unwrap();
        assert_eq!(open.rows, p.rows);
        let stored = &other.timelines[&kicks.id].clips[0];
        assert_eq!(stored.row, Some(4));
        // Switching to it shows its rows.
        let mut switched = p.clone();
        let id = other.id.clone();
        switched.capture_active_arrangement();
        switched.apply_arrangement(&id).unwrap();
        assert!(!switched.rows[4].muted);
    }

    /// A note held over the start of a clip trimmed there plays its tail,
    /// and an offset past every note (a field read as something else)
    /// does not silence the clip.
    #[test]
    fn a_trimmed_pattern_keeps_a_note_held_over_its_start() {
        let mut fl = song();
        let notes = &mut fl.pattern_notes[0].2;
        notes[0].length_ticks = 1_920;
        let set_offset = |fl: &mut FlProject, offset: u64| {
            let clip = fl
                .playlist_clips
                .iter_mut()
                .find(|c| matches!(c.content, FlClipContent::Pattern { .. }))
                .unwrap();
            if let FlClipContent::Pattern { offset_ticks, .. } = &mut clip.content {
                *offset_ticks = offset;
            }
        };
        set_offset(&mut fl, 480);
        let p = build_project(&fl, "s", &|p| Some(p.to_string())).project;
        let screech = channel_track(&p, "Screech");
        let ClipContent::Midi(midi) = &screech.clips[0].content else {
            panic!("expected notes");
        };
        let first = &midi.clip.notes[0];
        assert_eq!((first.start_tick, first.duration_ticks), (0, 1_440));

        set_offset(&mut fl, 1_065_353_216);
        let p = build_project(&fl, "s", &|p| Some(p.to_string())).project;
        let ClipContent::Midi(midi) = &channel_track(&p, "Screech").clips[0].content else {
            panic!("expected notes");
        };
        assert!(
            !midi.clip.notes.is_empty(),
            "a nonsense offset leaves the notes"
        );
    }

    #[test]
    fn a_pattern_clip_trimmed_at_its_start_plays_from_there() {
        let mut fl = song();
        let clip = fl
            .playlist_clips
            .iter_mut()
            .find(|c| matches!(c.content, FlClipContent::Pattern { .. }))
            .expect("the song places a pattern");
        let FlClipContent::Pattern { offset_ticks, .. } = &mut clip.content else {
            unreachable!()
        };
        *offset_ticks = hardwave_midi::PPQ;
        let whole = build_project(&song(), "s", &|p| Some(p.to_string())).project;
        let trimmed = build_project(&fl, "s", &|p| Some(p.to_string())).project;
        let first_notes = |p: &Project| -> Vec<u64> {
            p.tracks
                .iter()
                .flat_map(|t| &t.clips)
                .find_map(|c| match &c.content {
                    ClipContent::Midi(m) => {
                        Some(m.clip.notes.iter().map(|n| n.start_tick).collect())
                    }
                    _ => None,
                })
                .unwrap_or_default()
        };
        let before = first_notes(&whole);
        let after = first_notes(&trimmed);
        let expected: Vec<u64> = before
            .iter()
            .filter(|t| **t >= hardwave_midi::PPQ)
            .map(|t| t - hardwave_midi::PPQ)
            .collect();
        assert!(before.len() > after.len() || before.iter().all(|t| *t >= hardwave_midi::PPQ));
        assert_eq!(after, expected);
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

    /// A route to the master below full is the fader that much lower, as
    /// any insert's one route is; it used to play at full.
    #[test]
    fn a_route_to_the_master_below_full_lowers_the_fader() {
        let group = |level: i32| {
            let mut fl = mixer_song();
            let g = fl
                .mixer
                .iter_mut()
                .find(|m| m.name == "Group")
                .expect("a group");
            g.volume = 10_000;
            g.routes = vec![FlRoute { to: 0, level }];
            let p = build_project(&fl, "s", &|p| Some(p.to_string())).project;
            let t = p
                .tracks
                .iter()
                .find(|t| t.name == "Group")
                .expect("its track")
                .clone();
            (t.volume_db, t.output_bus)
        };
        let (full, to_full) = group(12_800);
        let (half, to_half) = group(6_400);
        assert_eq!(
            (to_full, to_half),
            (None, None),
            "both play into the master"
        );
        assert!(
            (half - (full + fl_volume_db(6_400))).abs() < 1e-9,
            "{full} {half}"
        );
    }

    /// A route on to the master below full, beside a send: the fader is
    /// lowered and the send raised by as much, so both are where FL had
    /// them. Only an insert with no other route was taken; a group at
    /// -6 dB with a reverb send played at full.
    #[test]
    fn a_master_route_below_full_beside_a_send_lowers_the_fader_and_keeps_the_send() {
        let mut fl = mixer_song();
        let kick = fl.mixer.iter_mut().find(|m| m.index == 2).unwrap();
        kick.routes = vec![
            FlRoute {
                to: 0,
                level: 6_400,
            },
            FlRoute {
                to: 5,
                level: 6_400,
            },
        ];
        let built = build_project(&fl, "s", &|p| Some(p.to_string()));
        let full = build_project(&mixer_song(), "s", &|p| Some(p.to_string()));
        let track = |b: &FlBuild| {
            b.project
                .tracks
                .iter()
                .find(|t| t.sends.len() == 1 && t.output_bus.is_none() && t.name != "Screech")
                .cloned()
                .expect("the kick insert")
        };
        let (lowered, before) = (track(&built), track(&full));
        let offset = fl_volume_db(6_400);
        assert!((lowered.volume_db - (before.volume_db + offset)).abs() < 1e-9);
        assert!(
            lowered.sends[0].gain_db.abs() < 1e-9,
            "the send at half, raised by the fader's drop, is where FL had it: {}",
            lowered.sends[0].gain_db
        );
        assert!(built.report.route_levels_left_behind.is_empty());
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
        // Named, so it comes, though nothing plays into it.
        let unused = by_name("Unused");
        assert!(matches!(unused.kind, TrackKind::Bus));
        assert!(p
            .tracks
            .iter()
            .all(|t| t.output_bus.as_deref() != Some(unused.id.as_str())));
        assert_eq!(built.report.mixer_inserts, 7);
        // Numbered as in FL.
        for (name, number) in [
            ("Screech", 1),
            ("Insert 2", 2),
            ("808", 3),
            ("Group", 4),
            ("Reverb", 5),
            ("Unused", 6),
            ("Ghost", 7),
        ] {
            assert_eq!(by_name(name).mixer_number, Some(number), "{name}");
            assert_eq!(by_name(name).rack_group, None, "{name} is no channel");
        }

        // The tracks play into their channels' inserts.
        assert_eq!(
            channel_track(p, "Screech").output_bus.as_ref(),
            Some(&screech.id)
        );
        assert_eq!(
            channel_track(p, "808 Kick").output_bus.as_ref(),
            Some(&eight.id)
        );
        assert_eq!(
            channel_track(p, "Kick render").output_bus.as_ref(),
            Some(&insert_2.id)
        );
        assert_eq!(
            channel_track(p, "Lost vocal").output_bus.as_ref(),
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

    /// An insert comes when it holds something of the song: a name, a
    /// colour, an effect, a fader away from 0 dB, or sound. FL's "current"
    /// insert never does, and an untouched one nothing reaches is only a
    /// number.
    #[test]
    fn inserts_come_numbered_as_in_fl_and_the_current_one_does_not() {
        let mut fl = mixer_song();
        let blank = |index: u32| FlMixerTrack {
            index,
            routes: vec![FlRoute {
                to: 0,
                level: 12_800,
            }],
            ..Default::default()
        };
        fl.mixer.extend([
            // Untouched.
            blank(8),
            FlMixerTrack {
                color: Some("#3b82f6".into()),
                ..blank(9)
            },
            FlMixerTrack {
                volume: 9_000,
                ..blank(10)
            },
            FlMixerTrack {
                slots: vec![crate::fl_import::FlMixerSlot {
                    index: 0,
                    plugin: "Fruity Limiter".into(),
                    name: None,
                    enabled: true,
                    mix: 12_800,
                    data: Vec::new(),
                }],
                ..blank(11)
            },
            // Only an insert that comes routes into it.
            blank(12),
            // FL's current insert, here with a channel pointed at it.
            FlMixerTrack {
                current: true,
                ..blank(13)
            },
        ]);
        fl.mixer[10].routes.push(FlRoute {
            to: 12,
            level: 6_400,
        });
        fl.channels[4].settings.mixer_insert = 13;
        let built = build_project(&fl, "Mixer", &here);
        let p = &built.project;
        let numbers: Vec<u16> = p.tracks.iter().filter_map(|t| t.mixer_number).collect();
        assert_eq!(numbers, [0, 1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12]);
        assert_eq!(insert_track(p, 9).color, "#3b82f6");
        assert_eq!(insert_track(p, 9).name, "Insert 9");
        assert!(insert_track(p, 10).volume_db < -6.0);
        assert_eq!(
            insert_track(p, 11).inserts.len(),
            0,
            "a limiter cannot come"
        );
        assert_eq!(insert_track(p, 10).sends[0].target, insert_track(p, 12).id);
        assert_eq!(
            channel_track(p, "Lost vocal").output_bus,
            None,
            "a channel on the current insert plays into the master"
        );
        assert_eq!(built.report.mixer_inserts, 11);
        // Opened as a song, every number stays where FL had it.
        let opened = Project::from_bytes(&p.to_bytes().unwrap()).unwrap();
        let again: Vec<u16> = opened
            .tracks
            .iter()
            .filter_map(|t| t.mixer_number)
            .collect();
        assert_eq!(again, numbers);
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

        let serum = channel_track(p, "Screech");
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

        let eight = channel_track(p, "808");
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
                arrangement: 0,
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
        let group = insert_track(p, 4);
        // Each sits on its own automation channel, on its FL row, moving
        // the knob of the track that has it.
        let on = |channel: &str| -> &crate::automation_clip::AutomationClip {
            let home = channel_track(p, channel);
            assert!(matches!(home.kind, TrackKind::Automation), "{channel}");
            assert!(home.automation_clips.iter().all(|c| c.row == Some(19)));
            home.automation_clips
                .iter()
                .find(|c| c.start_tick == 8 * beat)
                .unwrap_or_else(|| panic!("{channel} has its clip"))
        };
        let fader = on("Group - Volume");
        assert_eq!(fader.target, AutomationTarget::TrackVolume);
        assert_eq!(fader.target_track.as_ref(), Some(&group.id));
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
        let eq_freq = on("EQ - Group - Band 3 freq");
        assert!(
            matches!(&eq_freq.target, AutomationTarget::PluginParam { slot_id, param_id: 9 } if slot_id == eq_slot),
            "band 3's frequency, the built-in EQ's parameter 9"
        );
        assert_eq!(eq_freq.target_track.as_ref(), Some(&group.id));
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

        let serum = channel_track(p, "Screech");
        let macro_2 = on("Serum 2 - Macro 2");
        assert_eq!(
            macro_2.target,
            AutomationTarget::PluginParam {
                slot_id: serum.inserts[0].id.clone(),
                param_id: 7,
            }
        );
        assert_eq!(macro_2.target_track.as_ref(), Some(&serum.id));

        let kicks = channel_track(p, "Kick render");
        let volume = on("Kick render - Channel volume");
        assert_eq!(volume.target, AutomationTarget::TrackVolume);
        assert_eq!(volume.target_track.as_ref(), Some(&kicks.id));
        assert_eq!(volume.lane.points.len(), 2);
        // At the knob's default the fader is at 0 dB, where the track is.
        let at_default = Scale::AudioChannel.apply(10_000.0 / 12_800.0);
        assert!((at_default - 60.0 / 66.0).abs() < 1e-3);
        assert!(kicks.volume_db.abs() < 1e-9);

        // Channel pitch has nothing to move here: its clip sits on its row
        // moving nothing, and the report says so.
        let pitch = on("Screech - Channel pitch");
        assert_eq!(pitch.target_track, None);
        assert_eq!(
            built.report.automation_left_behind,
            vec!["Cutoff".to_string(), "Screech - Channel pitch".into()]
        );
        assert_eq!(built.report.automation_clips, 4);
        assert_eq!(built.report.automation_clips_moving_nothing, 2);
        // The muted placement is not there.
        assert!(p
            .tracks
            .iter()
            .flat_map(|t| &t.automation_clips)
            .all(|c| c.start_tick != 0 || c.row != Some(19)));
    }

    /// A sample channel's volume automation lands on the channel, also when
    /// its samples share a row with others': it had nowhere to go when a
    /// row was a track.
    #[test]
    fn channel_volume_automation_is_on_the_channel_whatever_its_row() {
        use crate::fl_import::{FlAutomationLink, FlAutomationTarget as T};
        let mut fl = song();
        // The lost vocal on the kick render's row.
        fl.playlist_clips[3].track_index = 5;
        let mut automation = channel(
            "Vocal - Channel volume",
            FlChannelKind::Automation,
            None,
            None,
        );
        automation.automation = vec![point(0.0, 0.0), point(1.0, 1.0)];
        fl.channels.push(automation);
        fl.automation_links.push(FlAutomationLink {
            channel: 5,
            target: T::ChannelVolume(4),
        });
        fl.playlist_clips.push(FlPlaylistClip {
            arrangement: 0,
            track_index: 7,
            start_tick: 0,
            length_ticks: 960,
            content: FlClipContent::Automation {
                channel_index: 5,
                target: "Vocal - Channel volume".into(),
                offset_ticks: 0,
            },
            muted: false,
        });
        let built = build_project(&fl, "s", &here);
        let p = &built.project;
        let clips = &channel_track(p, "Vocal - Channel volume").automation_clips;
        assert_eq!(clips.len(), 1);
        assert_eq!(
            clips[0].target_track.as_ref(),
            Some(&channel_track(p, "Lost vocal").id)
        );
        assert_eq!(clips[0].row, Some(6));
        assert_eq!(built.report.automation_clips, 1);
    }

    #[test]
    fn a_track_soloed_in_fl_is_a_soloed_row_and_the_rest_are_not_switched_off() {
        let mut fl = song();
        // The kick render's playlist track 5 soloed: FL switched every
        // other track off for it, the patterns' track 1 among them.
        fl.playlist_tracks_solo = vec![5];
        fl.playlist_tracks_off = vec![1, 9];
        let built = build_project(&fl, "Solo", &here);
        let p = &built.project;
        assert!(p.rows[4].soloed && !p.rows[4].muted);
        assert!(
            p.rows
                .iter()
                .enumerate()
                .all(|(i, r)| i == 4 || (!r.muted && !r.soloed)),
            "unsoloing gives them back"
        );
        assert!(p
            .tracks
            .iter()
            .all(|t| !t.soloed && !t.muted || t.name == "Screech"));
        let screech = channel_track(p, "Screech");
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
        let kicks = channel_track(&built.project, "Kick render");
        let ClipContent::Audio(clip) = &kicks.clips[0].content else {
            panic!("expected audio");
        };
        let beat = hardwave_midi::PPQ;
        assert_eq!(
            (clip.fade_in_ticks, clip.fade_out_ticks),
            (beat / 4, beat / 2)
        );
        // Its own volume; the channel's is on the channel.
        assert!((clip.gain_db - 20.0 * 0.631f64.log10()).abs() < 1e-6);
        assert!((20.0 * 0.631f64.log10() + 4.0).abs() < 0.01);

        // A fade longer than its clip fills the clip and no more.
        let mut long = song();
        if let FlClipContent::AudioSample { fade_out_ms, .. } = &mut long.playlist_clips[1].content
        {
            *fade_out_ms = 10_000.0;
        }
        let built = build_project(&long, "Fades", &here);
        let ClipContent::Audio(clip) =
            &channel_track(&built.project, "Kick render").clips[0].content
        else {
            panic!("expected audio");
        };
        assert_eq!(clip.fade_out_ticks, long.playlist_clips[1].length_ticks);
    }
}
