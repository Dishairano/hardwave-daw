//! Channel-rack channel model — the four channel types (Sampler,
//! AudioClip, AutomationClip, Layer), pattern-to-arrangement-clip
//! conversion, MIDI routing target resolution, pad preview helper,
//! and the project-wide "global write mode" flag.

use crate::automation::AutomationTarget;
use crate::clip::ClipId;
use serde::{Deserialize, Serialize};

/// One row in the channel rack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub mixer_track: Option<String>,
    pub midi_note: u8,
    pub color_argb: u32,
    pub muted: bool,
}

impl Channel {
    pub fn sampler(id: impl Into<String>, sample_path: impl Into<String>) -> Self {
        Self::new(
            id,
            ChannelKind::Sampler {
                sample_path: sample_path.into(),
            },
        )
    }

    pub fn audio_clip(id: impl Into<String>, clip_id: ClipId) -> Self {
        Self::new(id, ChannelKind::AudioClip { clip_id })
    }

    pub fn automation_clip(id: impl Into<String>, target: AutomationTarget) -> Self {
        Self::new(id, ChannelKind::AutomationClip { target })
    }

    pub fn layer(id: impl Into<String>, member_ids: Vec<String>) -> Self {
        Self::new(id, ChannelKind::Layer { member_ids })
    }

    fn new(id: impl Into<String>, kind: ChannelKind) -> Self {
        let id = id.into();
        let name = default_name_for(&kind, &id);
        Self {
            id,
            name,
            kind,
            mixer_track: None,
            midi_note: 60,
            color_argb: 0xFF_80_80_80,
            muted: false,
        }
    }
}

/// The four channel kinds the channel rack can hold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChannelKind {
    /// Loads a single audio file and triggers it note-per-step.
    Sampler { sample_path: String },
    /// References an existing audio clip on the arrangement — good
    /// for sidechain ducks, drum fills you've already rendered, etc.
    AudioClip { clip_id: ClipId },
    /// Targets an automation lane — steps draw points rather than
    /// triggering sound. Host draws a different row background.
    AutomationClip { target: AutomationTarget },
    /// Layer — triggering the channel dispatches to every member.
    Layer { member_ids: Vec<String> },
}

impl ChannelKind {
    pub fn is_audio_producing(&self) -> bool {
        matches!(
            self,
            ChannelKind::Sampler { .. } | ChannelKind::AudioClip { .. } | ChannelKind::Layer { .. }
        )
    }

    pub fn is_automation(&self) -> bool {
        matches!(self, ChannelKind::AutomationClip { .. })
    }
}

fn default_name_for(kind: &ChannelKind, id: &str) -> String {
    match kind {
        ChannelKind::Sampler { sample_path } => std::path::Path::new(sample_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| id.to_string()),
        ChannelKind::AudioClip { .. } => format!("Audio {}", id),
        ChannelKind::AutomationClip { .. } => format!("Automation {}", id),
        ChannelKind::Layer { .. } => format!("Layer {}", id),
    }
}

/// A compiled pattern step — which channel is triggered at which
/// position, with a per-step velocity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternStep {
    pub channel_id: String,
    pub step: u32,
    pub velocity: u8,
}

/// A channel-rack pattern — ordered list of steps plus length.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: String,
    pub name: String,
    pub length_steps: u32,
    pub steps_per_beat: u32,
    pub color_argb: u32,
    pub steps: Vec<PatternStep>,
}

impl Pattern {
    /// Convert the pattern into a list of arrangement clip placements
    /// — one audio/automation clip per armed channel, positioned at
    /// `timeline_start_tick`, with length derived from the pattern's
    /// step count + steps-per-beat and the project ticks-per-beat.
    pub fn to_arrangement_clips(
        &self,
        timeline_start_tick: u64,
        ticks_per_beat: u64,
        channels: &[Channel],
    ) -> Vec<ArrangementClipPlacement> {
        if self.steps_per_beat == 0 || self.length_steps == 0 {
            return Vec::new();
        }
        let ticks_per_step = ticks_per_beat / self.steps_per_beat as u64;
        let length_ticks = ticks_per_step * self.length_steps as u64;
        let mut out: Vec<ArrangementClipPlacement> = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for step in &self.steps {
            if seen.contains(&step.channel_id.as_str()) {
                continue;
            }
            if let Some(channel) = channels.iter().find(|c| c.id == step.channel_id) {
                seen.push(channel.id.as_str());
                out.push(ArrangementClipPlacement {
                    channel_id: channel.id.clone(),
                    timeline_start_tick,
                    length_ticks,
                    kind: channel.kind.clone(),
                });
            }
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct ArrangementClipPlacement {
    pub channel_id: String,
    pub timeline_start_tick: u64,
    pub length_ticks: u64,
    pub kind: ChannelKind,
}

/// Resolve a MIDI event to the target channel. Rules:
///
/// 1. If `selected_channel_id` is `Some`, that channel wins — route
///    all MIDI regardless of note.
/// 2. Otherwise, find the channel whose `midi_note` matches the
///    incoming note.
/// 3. Otherwise, return `None` (drop the event).
pub fn resolve_midi_route<'a>(
    channels: &'a [Channel],
    selected_channel_id: Option<&str>,
    incoming_note: u8,
) -> Option<&'a Channel> {
    if let Some(id) = selected_channel_id {
        return channels.iter().find(|c| c.id == id);
    }
    channels
        .iter()
        .find(|c| c.midi_note == incoming_note && !c.muted)
}

/// Pad preview — synthesize a deterministic filler sample sequence
/// so the UI has something to play when the user clicks a pad
/// without a sample loaded (and has something to show during
/// loading). Produces a short filtered noise burst.
pub fn pad_preview_samples(sample_rate: f32, duration_secs: f32) -> Vec<f32> {
    let n = ((duration_secs * sample_rate).max(1.0)) as usize;
    let mut rng: u32 = 0xDEAD_BEEF;
    let mut prev = 0.0_f32;
    let cutoff_alpha = 0.08;
    (0..n)
        .map(|i| {
            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let raw = ((rng >> 8) as f32 / 8_388_608.0) - 1.0;
            prev += cutoff_alpha * (raw - prev);
            let t = i as f32 / n as f32;
            let env = (1.0 - t).powi(3); // fast decay
            prev * env
        })
        .collect()
}

/// Project-wide automation write mode — the toolbar's global button
/// toggles this. Per-track write-mode toggles are OR-combined with
/// this value so "global write = on" overrides per-track settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GlobalWriteMode {
    pub enabled: bool,
}

impl GlobalWriteMode {
    pub fn combined(&self, per_track: bool) -> bool {
        self.enabled || per_track
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampler_channel_uses_filename_as_name() {
        let c = Channel::sampler("c1", "/samples/808/kick.wav");
        assert_eq!(c.name, "kick");
    }

    #[test]
    fn channel_kinds_classify_correctly() {
        assert!(Channel::sampler("s", "x").kind.is_audio_producing());
        assert!(!Channel::sampler("s", "x").kind.is_automation());
        assert!(Channel::automation_clip("a", AutomationTarget::TrackVolume)
            .kind
            .is_automation());
        assert!(Channel::layer("l", vec!["a".into()])
            .kind
            .is_audio_producing());
    }

    #[test]
    fn pattern_converts_to_arrangement_clips() {
        let channels = vec![
            Channel::sampler("c1", "/x/kick.wav"),
            Channel::sampler("c2", "/x/snare.wav"),
        ];
        let pattern = Pattern {
            id: "p1".into(),
            name: "Beat 1".into(),
            length_steps: 16,
            steps_per_beat: 4,
            color_argb: 0xFF_00_80_FF,
            steps: vec![
                PatternStep {
                    channel_id: "c1".into(),
                    step: 0,
                    velocity: 100,
                },
                PatternStep {
                    channel_id: "c2".into(),
                    step: 4,
                    velocity: 100,
                },
                PatternStep {
                    channel_id: "c1".into(),
                    step: 8,
                    velocity: 100,
                },
            ],
        };
        let clips = pattern.to_arrangement_clips(1_000, 480, &channels);
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].timeline_start_tick, 1_000);
        // 16 steps at 4 steps/beat = 4 beats → 4 × 480 = 1920 ticks.
        assert_eq!(clips[0].length_ticks, 1_920);
    }

    #[test]
    fn empty_pattern_returns_no_clips() {
        let channels = vec![Channel::sampler("c1", "x")];
        let empty = Pattern {
            id: "p".into(),
            name: "".into(),
            length_steps: 0,
            steps_per_beat: 4,
            color_argb: 0,
            steps: Vec::new(),
        };
        assert!(empty.to_arrangement_clips(0, 480, &channels).is_empty());
    }

    #[test]
    fn midi_routes_to_selected_channel_first() {
        let channels = vec![Channel::sampler("c1", "x"), Channel::sampler("c2", "y")];
        let route = resolve_midi_route(&channels, Some("c2"), 60).unwrap();
        assert_eq!(route.id, "c2");
    }

    #[test]
    fn midi_routes_by_note_when_no_selection() {
        let mut c1 = Channel::sampler("c1", "x");
        c1.midi_note = 36;
        let mut c2 = Channel::sampler("c2", "y");
        c2.midi_note = 38;
        let channels = vec![c1, c2];
        assert_eq!(resolve_midi_route(&channels, None, 38).unwrap().id, "c2");
        assert_eq!(resolve_midi_route(&channels, None, 36).unwrap().id, "c1");
        assert!(resolve_midi_route(&channels, None, 40).is_none());
    }

    #[test]
    fn midi_skips_muted_channels() {
        let mut c = Channel::sampler("c1", "x");
        c.midi_note = 60;
        c.muted = true;
        let channels = vec![c];
        assert!(resolve_midi_route(&channels, None, 60).is_none());
    }

    #[test]
    fn pad_preview_produces_nonzero_decaying_signal() {
        let samples = pad_preview_samples(48_000.0, 0.1);
        assert!(!samples.is_empty());
        let peak: f32 = samples.iter().fold(0.0, |acc, v| acc.max(v.abs()));
        assert!(peak > 0.0);
        let first_half: f32 = samples[..samples.len() / 2].iter().map(|v| v * v).sum();
        let second_half: f32 = samples[samples.len() / 2..].iter().map(|v| v * v).sum();
        assert!(first_half > second_half, "preview should decay");
    }

    #[test]
    fn global_write_mode_or_combines_with_per_track() {
        let off = GlobalWriteMode { enabled: false };
        let on = GlobalWriteMode { enabled: true };
        assert!(!off.combined(false));
        assert!(off.combined(true));
        assert!(on.combined(false));
        assert!(on.combined(true));
    }
}
