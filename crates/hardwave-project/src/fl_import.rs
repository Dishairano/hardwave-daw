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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlClipContent {
    Pattern { pattern_index: u32 },
    AudioSample { sample_path: String },
    Automation { target: String },
}

/// Parsed mixer track — volume / pan / routing only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlMixerTrack {
    pub name: String,
    pub volume_db: f32,
    pub pan: f32,
    pub muted: bool,
    pub routes_to: Vec<u32>,
}

/// Parsed `.flp` project — the raw import result.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlProject {
    pub bpm: f32,
    pub time_sig_numerator: u8,
    pub time_sig_denominator: u8,
    pub channels: Vec<FlChannel>,
    pub notes: Vec<(u32, Vec<FlNote>)>, // (channel_index, notes)
    pub playlist_clips: Vec<FlPlaylistClip>,
    pub mixer: Vec<FlMixerTrack>,
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
                },
                FlChannel {
                    name: "Sytrus Lead".into(),
                    sample_path: None,
                    plugin_name: Some("Sytrus".into()),
                    pattern_steps: vec![true; 16],
                },
                FlChannel {
                    name: "Harmor Pad".into(),
                    sample_path: None,
                    plugin_name: Some("Harmor".into()),
                    pattern_steps: vec![false; 16],
                },
                FlChannel {
                    name: "Mystery Plugin".into(),
                    sample_path: None,
                    plugin_name: Some("WeirdOne".into()),
                    pattern_steps: vec![false; 16],
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
            }],
            mixer: vec![FlMixerTrack {
                name: "Master".into(),
                volume_db: -6.0,
                pan: 0.0,
                muted: false,
                routes_to: Vec::new(),
            }],
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
                target: "master-volume".into(),
            },
        });
        let report = summarize_import(&project, &default_plugin_mappings());
        assert_eq!(report.imported_automation_clips, 1);
    }
}
