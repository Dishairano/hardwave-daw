//! FL Studio `.flp` import — parser contract + import report.
//!
//! This module defines the data shape the importer emits on a
//! best-effort parse of FL Studio project files. The binary parser
//! lives in a sibling module / worker crate; here we define the
//! types the UI consumes and the plugin-mapping table it uses to
//! translate FL natives into Hardwave equivalents.

use serde::{Deserialize, Serialize};

/// Parsed channel-rack channel from a `.flp` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlChannel {
    pub name: String,
    pub sample_path: Option<String>,
    pub plugin_name: Option<String>,
    pub pattern_steps: Vec<bool>,
    /// What the channel is in FL, which decides what it becomes here.
    #[serde(default)]
    pub kind: FlChannelKind,
    /// Its level, pan, pitch and sample settings.
    #[serde(default)]
    pub settings: FlChannelSettings,
}

/// A channel's settings as FL stores them, in FL's own units. What they
/// mean here is worked out where the song is built (fl_build.rs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlChannelSettings {
    /// The channel's on/off switch in the rack.
    pub enabled: bool,
    /// The volume knob, 0 to 12800; 10000 is FL's 78 % default.
    pub volume: u32,
    /// The pan knob, 0 (left) to 12800 (right); 6400 is the centre.
    pub pan: i32,
    /// The pitch knob, in cents.
    pub pitch_cents: i32,
    /// The mixer insert it plays into: 0 is the master, -1 "current".
    pub mixer_insert: i32,
    /// Sample settings: played backwards.
    pub reversed: bool,
    /// Sample settings: left and right swapped.
    pub swap_stereo: bool,
    /// Sample settings: polarity inverted.
    pub polarity_inverted: bool,
    /// The root note, as a MIDI key; FL's default is 60 (its C5).
    pub root_note: u8,
    /// FL's quick fade knobs on the sample, 0 to 1024 (0 is off).
    pub fade_in: u16,
    pub fade_out: u16,
    pub stretch: FlStretch,
}

impl Default for FlChannelSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 10_000,
            pan: 6_400,
            pitch_cents: 0,
            mixer_insert: -1,
            reversed: false,
            swap_stereo: false,
            polarity_inverted: false,
            root_note: 60,
            fade_in: 0,
            fade_out: 0,
            stretch: FlStretch::default(),
        }
    }
}

/// The time stretching section of FL's sample settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FlStretch {
    /// The TIME knob: how long the whole sample should last, in our ticks.
    /// 0 is "(none)", the sample at its own length.
    pub time_ticks: f64,
    /// The PITCH knob, in cents.
    pub pitch_cents: i32,
    /// The MUL knob, as a factor on the length (0.25 to 4).
    pub multiplier: f64,
    /// The mode: 0 is Resample, where length and pitch change together like
    /// a tape; every other mode keeps them apart.
    pub mode: i32,
}

impl Default for FlStretch {
    fn default() -> Self {
        Self {
            time_ticks: 0.0,
            pitch_cents: 0,
            multiplier: 1.0,
            mode: 0,
        }
    }
}

impl FlStretch {
    /// Resample: speed and pitch move together.
    pub fn is_resample(&self) -> bool {
        self.mode == 0
    }
}

/// The kinds of channel in FL's channel rack, as the file numbers them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlChannelKind {
    /// Plays a sample from its notes.
    Sampler,
    /// A plug-in instrument (FL's own, or a VST in FL's wrapper).
    Instrument,
    /// Several channels played as one.
    Layer,
    /// An audio clip: a file placed on the playlist.
    AudioClip,
    /// An automation clip.
    Automation,
    /// Anything else, or not said.
    #[default]
    Other,
}

impl FlChannelKind {
    /// From the byte FL writes.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Sampler,
            2 => Self::Instrument,
            3 => Self::Layer,
            4 => Self::AudioClip,
            5 => Self::Automation,
            _ => Self::Other,
        }
    }
}

/// Parsed piano-roll note.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FlNote {
    pub tick: u64,
    pub length_ticks: u64,
    pub pitch: u8,
    pub velocity: u8,
}

/// Parsed arrangement / playlist clip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlPlaylistClip {
    pub track_index: u32,
    pub start_tick: u64,
    pub length_ticks: u64,
    pub content: FlClipContent,
    /// Muted on the playlist (FL greys it out).
    #[serde(default)]
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlClipContent {
    Pattern {
        pattern_index: u32,
    },
    /// A sample on the playlist: the channel it belongs to, its file, and
    /// where in the file the clip starts and ends, in milliseconds (FL
    /// writes -1 for "not trimmed").
    AudioSample {
        channel_index: u32,
        sample_path: String,
        start_offset_ms: f32,
        end_offset_ms: f32,
    },
    Automation {
        channel_index: u32,
        target: String,
    },
}

/// An insert in FL's mixer: where channels play into, where the effects
/// sit, and where it sends its sound on. Values are FL's own; the import
/// turns them into ours.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlMixerTrack {
    /// Its number in FL's mixer; 0 is the master.
    pub index: u32,
    /// Empty when it has FL's default name ("Insert 3").
    pub name: String,
    /// Off is FL's mute switch on the insert.
    pub enabled: bool,
    /// All its effect slots on or bypassed at once.
    pub effects_enabled: bool,
    pub polarity_inverted: bool,
    pub swap_stereo: bool,
    pub solo: bool,
    /// The fader, 12800 at 100 % (0 dB), 16000 at the top.
    pub volume: i32,
    /// The pan knob, -6400 (left) to 6400 (right).
    pub pan: i32,
    /// Stereo separation: 64 is mono, 0 as recorded, -64 fully separated.
    pub stereo_separation: i32,
    /// The gains of the insert's own three-band EQ, in hundredths of a dB.
    pub eq_gains: [i32; 3],
    /// Where it sends its sound, with the level of each.
    pub routes: Vec<FlRoute>,
    /// The effect slots that hold a plug-in.
    pub slots: Vec<FlMixerSlot>,
}

impl Default for FlMixerTrack {
    fn default() -> Self {
        Self {
            index: 0,
            name: String::new(),
            enabled: true,
            effects_enabled: true,
            polarity_inverted: false,
            swap_stereo: false,
            solo: false,
            volume: 12_800,
            pan: 0,
            stereo_separation: 0,
            eq_gains: [0; 3],
            routes: Vec::new(),
            slots: Vec::new(),
        }
    }
}

/// A route from one insert to another.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FlRoute {
    /// The insert it goes to; 0 is the master.
    pub to: u32,
    /// The send level, on the fader's scale (12800 is 0 dB). 0 carries no
    /// sound: FL makes those for sidechains.
    pub level: i32,
}

/// An effect slot in an insert, with the plug-in in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlMixerSlot {
    /// Its place in the insert, 0 to 9.
    pub index: u32,
    /// What FL calls the plug-in ("Fruity Parametric EQ 2"; "Fruity
    /// Wrapper" for anyone else's).
    pub plugin: String,
    /// The name it was given, when it has one.
    pub name: Option<String>,
    pub enabled: bool,
    /// Dry/wet, 0 to 12800 (fully wet).
    pub mix: i32,
    /// The plug-in's own data: its settings, in its own format.
    #[serde(skip)]
    pub data: Vec<u8>,
}

impl FlMixerSlot {
    /// The name to show: the one it was given, the name of the plug-in in
    /// FL's wrapper, or FL's own name for it.
    pub fn label(&self) -> String {
        self.name
            .clone()
            .or_else(|| (self.plugin == WRAPPER).then(|| wrapped_plugin_name(&self.data))?)
            .unwrap_or_else(|| self.plugin.clone())
    }
}

/// What FL calls the wrapper it puts anyone else's plug-in in.
pub const WRAPPER: &str = "Fruity Wrapper";

/// The name of the plug-in in FL's wrapper. The wrapper's data is four
/// bytes, then chunks: a kind (four bytes), a length (eight) and that many
/// bytes. Kind 54 is the plug-in's name.
pub fn wrapped_plugin_name(data: &[u8]) -> Option<String> {
    const NAME: u32 = 54;
    let mut at = 4usize;
    while let Some(head) = data.get(at..at + 12) {
        let kind = u32::from_le_bytes(head[0..4].try_into().ok()?);
        let len = usize::try_from(u64::from_le_bytes(head[4..12].try_into().ok()?)).ok()?;
        at += 12;
        let body = data.get(at..at.checked_add(len)?)?;
        if kind == NAME {
            let name = String::from_utf8_lossy(body)
                .trim_end_matches('\0')
                .trim()
                .to_string();
            return (!name.is_empty()).then_some(name);
        }
        at += len;
    }
    None
}

/// Parsed `.flp` project — the raw import result.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlProject {
    pub bpm: f32,
    pub time_sig_numerator: u8,
    pub time_sig_denominator: u8,
    pub channels: Vec<FlChannel>,
    pub notes: Vec<(u32, Vec<FlNote>)>, // (channel_index, notes)
    /// The same notes, but kept per pattern as well, which is what
    /// the playlist needs: a pattern is placed many times, and each
    /// placement brings its channels' notes with it.
    #[serde(default)]
    pub pattern_notes: Vec<(u32, u32, Vec<FlNote>)>, // (pattern, channel, notes)
    /// Pattern names, for naming the clips they become.
    #[serde(default)]
    pub pattern_names: Vec<(u32, String)>,
    pub playlist_clips: Vec<FlPlaylistClip>,
    pub mixer: Vec<FlMixerTrack>,
    /// Names given to playlist tracks, by track number (1 is the top).
    #[serde(default)]
    pub playlist_track_names: Vec<(u32, String)>,
    /// Playlist tracks switched off in FL, by track number.
    #[serde(default)]
    pub playlist_tracks_off: Vec<u32>,
}

/// Plugin mapping — translate an FL native plugin name into a
/// Hardwave equivalent. `None` on the right means "no native
/// equivalent; flag for user to resolve".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMap {
    pub from_fl: String,
    pub to_hardwave: Option<String>,
}

/// Built-in mapping table — maps the well-known FL natives to the
/// closest Hardwave plugin.
pub fn default_plugin_mappings() -> Vec<PluginMap> {
    vec![
        PluginMap {
            from_fl: "Sytrus".into(),
            to_hardwave: Some("hardwave-fm".into()),
        },
        PluginMap {
            from_fl: "3xOsc".into(),
            to_hardwave: Some("hardwave-subtractive".into()),
        },
        PluginMap {
            from_fl: "FPC".into(),
            to_hardwave: Some("hardwave-drum-machine".into()),
        },
        PluginMap {
            from_fl: "Fruity Kick".into(),
            to_hardwave: Some("hardwave-drum-synth".into()),
        },
        PluginMap {
            from_fl: "Fruity Limiter".into(),
            to_hardwave: Some("hardwave-limiter".into()),
        },
        PluginMap {
            from_fl: "Fruity Parametric EQ 2".into(),
            to_hardwave: Some("hardwave-parametric-eq".into()),
        },
        PluginMap {
            from_fl: "Fruity Delay 3".into(),
            to_hardwave: Some("hardwave-delay".into()),
        },
        PluginMap {
            from_fl: "Fruity Reverb 2".into(),
            to_hardwave: Some("hardwave-reverb".into()),
        },
        PluginMap {
            from_fl: "Harmor".into(),
            to_hardwave: None, // no equivalent
        },
    ]
}

/// One row in the user-facing import report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportFinding {
    pub severity: FindingSeverity,
    pub category: FindingCategory,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FindingSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FindingCategory {
    ChannelRack,
    PianoRoll,
    Playlist,
    Mixer,
    Automation,
    Plugin,
    Tempo,
    Unsupported,
}

/// Human-readable summary of what was imported and what wasn't —
/// shown to the user after the import finishes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportReport {
    pub findings: Vec<ImportFinding>,
    pub imported_channels: u32,
    pub imported_notes: u32,
    pub imported_playlist_clips: u32,
    pub imported_mixer_tracks: u32,
    pub imported_automation_clips: u32,
    pub skipped_plugins: Vec<String>,
}

impl ImportReport {
    pub fn info(&mut self, category: FindingCategory, message: impl Into<String>) {
        self.findings.push(ImportFinding {
            severity: FindingSeverity::Info,
            category,
            message: message.into(),
        });
    }

    pub fn warn(&mut self, category: FindingCategory, message: impl Into<String>) {
        self.findings.push(ImportFinding {
            severity: FindingSeverity::Warning,
            category,
            message: message.into(),
        });
    }

    pub fn error(&mut self, category: FindingCategory, message: impl Into<String>) {
        self.findings.push(ImportFinding {
            severity: FindingSeverity::Error,
            category,
            message: message.into(),
        });
    }

    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.severity == FindingSeverity::Error)
    }

    pub fn warning_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == FindingSeverity::Warning)
            .count()
    }
}

/// Simulated end-to-end summary pass — given a parsed `FlProject`
/// and the plugin mapping table, produce an `ImportReport` for the
/// UI. The actual arrangement + channel-rack mutation happens
/// elsewhere; this helper keeps the reporting logic testable in
/// isolation.
pub fn summarize_import(project: &FlProject, mappings: &[PluginMap]) -> ImportReport {
    let mut report = ImportReport {
        imported_channels: project.channels.len() as u32,
        imported_notes: project.notes.iter().map(|(_, n)| n.len() as u32).sum(),
        imported_playlist_clips: project.playlist_clips.len() as u32,
        imported_mixer_tracks: project.mixer.len() as u32,
        imported_automation_clips: project
            .playlist_clips
            .iter()
            .filter(|c| matches!(c.content, FlClipContent::Automation { .. }))
            .count() as u32,
        ..Default::default()
    };
    report.info(
        FindingCategory::Tempo,
        format!(
            "Tempo {} BPM, {}/{}",
            project.bpm, project.time_sig_numerator, project.time_sig_denominator
        ),
    );
    for channel in &project.channels {
        if let Some(plugin) = &channel.plugin_name {
            match map_plugin(plugin, mappings) {
                PluginMapResult::Equivalent(target) => {
                    report.info(
                        FindingCategory::Plugin,
                        format!("Mapped {} → {}", plugin, target),
                    );
                }
                PluginMapResult::NoEquivalent => {
                    report.warn(
                        FindingCategory::Plugin,
                        format!("{} has no Hardwave equivalent — channel muted until you assign a plugin", plugin),
                    );
                    report.skipped_plugins.push(plugin.clone());
                }
                PluginMapResult::Unknown => {
                    report.warn(
                        FindingCategory::Unsupported,
                        format!("Unknown plugin reference: {} (skipped)", plugin),
                    );
                    report.skipped_plugins.push(plugin.clone());
                }
            }
        }
    }
    report
}

#[derive(Debug, Clone, PartialEq)]
enum PluginMapResult {
    Equivalent(String),
    NoEquivalent,
    Unknown,
}

fn map_plugin(fl_name: &str, mappings: &[PluginMap]) -> PluginMapResult {
    if let Some(m) = mappings.iter().find(|m| m.from_fl == fl_name) {
        match &m.to_hardwave {
            Some(target) => PluginMapResult::Equivalent(target.clone()),
            None => PluginMapResult::NoEquivalent,
        }
    } else {
        PluginMapResult::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_project() -> FlProject {
        FlProject {
            bpm: 140.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            channels: vec![
                FlChannel {
                    name: "Kick".into(),
                    sample_path: Some("/samples/kick.wav".into()),
                    plugin_name: None,
                    pattern_steps: [true, false, false, false].repeat(4),
                    kind: FlChannelKind::Other,
                    settings: FlChannelSettings::default(),
                },
                FlChannel {
                    name: "Sytrus Lead".into(),
                    sample_path: None,
                    plugin_name: Some("Sytrus".into()),
                    pattern_steps: vec![true; 16],
                    kind: FlChannelKind::Other,
                    settings: FlChannelSettings::default(),
                },
                FlChannel {
                    name: "Harmor Pad".into(),
                    sample_path: None,
                    plugin_name: Some("Harmor".into()),
                    pattern_steps: vec![false; 16],
                    kind: FlChannelKind::Other,
                    settings: FlChannelSettings::default(),
                },
                FlChannel {
                    name: "Mystery Plugin".into(),
                    sample_path: None,
                    plugin_name: Some("WeirdOne".into()),
                    pattern_steps: vec![false; 16],
                    kind: FlChannelKind::Other,
                    settings: FlChannelSettings::default(),
                },
            ],
            notes: vec![(
                1,
                vec![FlNote {
                    tick: 0,
                    length_ticks: 480,
                    pitch: 60,
                    velocity: 100,
                }],
            )],
            playlist_clips: vec![FlPlaylistClip {
                track_index: 0,
                start_tick: 0,
                length_ticks: 1920,
                content: FlClipContent::Pattern { pattern_index: 0 },
                muted: false,
            }],
            mixer: vec![FlMixerTrack {
                name: "Master".into(),
                ..Default::default()
            }],
            pattern_notes: Vec::new(),
            pattern_names: Vec::new(),
            playlist_track_names: Vec::new(),
            playlist_tracks_off: Vec::new(),
        }
    }

    #[test]
    fn summary_counts_aggregate_from_project() {
        let project = sample_project();
        let mappings = default_plugin_mappings();
        let report = summarize_import(&project, &mappings);
        assert_eq!(report.imported_channels, 4);
        assert_eq!(report.imported_notes, 1);
        assert_eq!(report.imported_playlist_clips, 1);
        assert_eq!(report.imported_mixer_tracks, 1);
    }

    #[test]
    fn summary_maps_sytrus_to_hardwave_fm() {
        let project = sample_project();
        let mappings = default_plugin_mappings();
        let report = summarize_import(&project, &mappings);
        assert!(report
            .findings
            .iter()
            .any(|f| f.message.contains("Sytrus") && f.message.contains("hardwave-fm")));
    }

    #[test]
    fn summary_warns_on_unknown_plugin_and_adds_to_skipped() {
        let project = sample_project();
        let mappings = default_plugin_mappings();
        let report = summarize_import(&project, &mappings);
        assert!(report.skipped_plugins.iter().any(|p| p == "WeirdOne"));
        assert!(report.skipped_plugins.iter().any(|p| p == "Harmor"));
        assert!(report.warning_count() >= 2);
        assert!(!report.has_errors());
    }

    #[test]
    fn default_mappings_contain_core_fl_natives() {
        let m = default_plugin_mappings();
        assert!(m.iter().any(|p| p.from_fl == "Sytrus"));
        assert!(m.iter().any(|p| p.from_fl == "3xOsc"));
        assert!(m.iter().any(|p| p.from_fl == "Fruity Limiter"));
    }

    #[test]
    fn report_severity_accessors() {
        let mut r = ImportReport::default();
        r.info(FindingCategory::Tempo, "ok");
        r.warn(FindingCategory::Plugin, "meh");
        r.error(FindingCategory::ChannelRack, "bad");
        assert!(r.has_errors());
        assert_eq!(r.warning_count(), 1);
    }

    #[test]
    fn automation_clip_count_derived_from_content_kind() {
        let mut project = sample_project();
        project.playlist_clips.push(FlPlaylistClip {
            track_index: 1,
            start_tick: 0,
            length_ticks: 960,
            content: FlClipContent::Automation {
                channel_index: 0,
                target: "master-volume".into(),
            },
            muted: false,
        });
        let report = summarize_import(&project, &default_plugin_mappings());
        assert_eq!(report.imported_automation_clips, 1);
    }

    /// FL's wrapper data with these chunks after its four leading bytes.
    fn wrapper(chunks: &[(u32, &[u8])]) -> Vec<u8> {
        let mut out = vec![12, 0, 0, 0];
        for (kind, body) in chunks {
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&(body.len() as u64).to_le_bytes());
            out.extend_from_slice(body);
        }
        out
    }

    #[test]
    fn a_wrapped_plug_in_goes_by_its_own_name() {
        let slot = |name: Option<&str>, data| FlMixerSlot {
            index: 0,
            plugin: WRAPPER.into(),
            name: name.map(String::from),
            enabled: true,
            mix: 12_800,
            data,
        };
        let data = wrapper(&[(1, &[0; 4]), (54, b"Serum2"), (56, b"Xfer Records")]);
        assert_eq!(slot(None, data.clone()).label(), "Serum2");
        assert_eq!(slot(Some("Lead synth"), data).label(), "Lead synth");
        // Data that says it is longer than it is gives no name, not a crash.
        let mut lying = wrapper(&[(54, b"Serum2")]);
        lying[8] = 0xFF;
        assert_eq!(slot(None, lying).label(), WRAPPER);
        assert_eq!(slot(None, Vec::new()).label(), WRAPPER);
    }
}
