use serde::{Deserialize, Serialize};

use crate::track::TrackId;

pub type ClipId = String;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FadeCurve {
    #[default]
    Linear,
    EqualPower,
    SCurve,
    Logarithmic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioClip {
    pub id: ClipId,
    pub name: String,
    /// Path to audio file (relative to project media/ dir).
    pub source_path: String,
    /// SHA-256 hash of source file for integrity.
    pub source_hash: String,
    /// Offset into source file in samples.
    pub source_start: u64,
    /// End position in source file in samples.
    pub source_end: u64,
    /// Clip gain in dB.
    pub gain_db: f64,
    /// Fade in length in ticks.
    pub fade_in_ticks: u64,
    /// Fade out length in ticks.
    pub fade_out_ticks: u64,
    pub muted: bool,
    /// Play this clip with the source read backwards.
    #[serde(default)]
    pub reversed: bool,
    /// Pitch shift in semitones. Combined with stretch via resampling.
    #[serde(default)]
    pub pitch_semitones: f64,
    /// Time-stretch ratio. 1.0 = realtime, 2.0 = half speed (longer), 0.5 = double speed.
    #[serde(default = "default_stretch_ratio")]
    pub stretch_ratio: f64,
    #[serde(default)]
    pub fade_in_curve: FadeCurve,
    #[serde(default)]
    pub fade_out_curve: FadeCurve,
}

fn default_stretch_ratio() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidiClipRef {
    pub id: ClipId,
    pub clip: hardwave_midi::MidiClip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClipContent {
    Audio(AudioClip),
    Midi(MidiClipRef),
}

/// A clip placed on a track at a specific position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipPlacement {
    pub content: ClipContent,
    pub track_id: TrackId,
    /// Position on the timeline in ticks (960 PPQ).
    pub position_ticks: u64,
    /// Length on the timeline in ticks (may differ from source length for time-stretched clips).
    pub length_ticks: u64,
    /// Lane index for comping (0 = main lane).
    pub lane: u32,
}
