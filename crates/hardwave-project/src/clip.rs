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
    /// Key into the engine's audio pool. Despite the name this is an
    /// opaque id, not a location: `import_audio_file` stores what
    /// `load_audio_file` returned, and playback looks the buffer up by it.
    /// Keeping it stable is what lets a moved file be relinked without
    /// touching any clip. The file it came from is `source_file`.
    pub source_path: String,
    /// SHA-256 of the source file at import, or empty for clips written
    /// before it was recorded. Identifies a file that moved or was renamed.
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
    /// Transient-anchored warp markers: a piecewise-linear timeline→source
    /// map. Each marker pins `clip_tick` (relative to the clip start, 960
    /// PPQ) to `source_sample` in the audio file; playback interpolates
    /// linearly between neighbouring markers. Kept sorted by `clip_tick`,
    /// unique per tick. Empty = classic single-ratio stretch via
    /// `stretch_ratio` (legacy projects deserialize to empty).
    #[serde(default)]
    pub warp_markers: Vec<WarpMarker>,
    /// Where the audio actually lives on disk.
    ///
    /// Without this a saved project had no record of its samples at all:
    /// `source_path` holds a pool id, so reopening a project tried to open a
    /// file named after a hash, every source failed to load, and every audio
    /// clip came back silent. Empty on projects written before this field
    /// existed, which the engine falls back to handling by treating
    /// `source_path` as a path.
    ///
    /// MUST STAY LAST, like `Project::timeline_state`: .hwp is MessagePack
    /// written by `rmp_serde::to_vec`, which encodes a struct positionally, so
    /// a field inserted anywhere else shifts every field after it.
    #[serde(default)]
    pub source_file: String,
}

/// One warp anchor: timeline tick ↔ source sample.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WarpMarker {
    pub clip_tick: u64,
    pub source_sample: u64,
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
