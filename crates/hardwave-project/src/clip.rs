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
    /// Fields added later come after this one, like `Project::timeline_state`:
    /// .hwp is MessagePack written by `rmp_serde::to_vec`, which encodes a
    /// struct positionally, so a field inserted before an older one shifts
    /// every field after it.
    #[serde(default)]
    pub source_file: String,
    /// What is done to the sample as it plays: its pan, polarity, stereo
    /// swap, normalising, DC offset and whether it plays like a tape. FL keeps
    /// these per channel; here they belong to the clip.
    #[serde(default)]
    pub treatment: SampleTreatment,
}

/// What is done to a clip's sample as it plays, on top of its gain, pitch,
/// stretch and fades. All off is the sample as recorded.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SampleTreatment {
    /// -1 is left, 1 right.
    pub pan: f64,
    pub polarity_inverted: bool,
    pub swap_stereo: bool,
    /// Brought up so its loudest peak reaches full scale.
    pub normalize: bool,
    /// Each channel's average taken off, so it sits on zero.
    pub remove_dc: bool,
    /// Speed and pitch move together, like a tape: a longer clip plays
    /// lower. Off, length and pitch are set apart.
    pub resample: bool,
    /// The left channel fades out and the right fades in over the sample.
    #[serde(default)]
    pub fade_stereo: bool,
    /// Where the sample begins, as a share of the file (0 = its first
    /// frame): what comes before is cut away, as FL's SMP START does.
    #[serde(default)]
    pub start: f64,
    /// How much of what remains after `start` is the sample, 0..1.
    #[serde(default = "one")]
    pub length: f64,
    /// Cuts the sample's quiet tail. 0 is off; from there the threshold
    /// rises from -60 dB under the peak (0.01) to the peak itself (1).
    #[serde(default)]
    pub trim: f64,
    /// The tiny fades at a clip's edges when it has none of its own.
    #[serde(default)]
    pub declick: DeclickMode,
}

fn one() -> f64 {
    1.0
}

impl Default for SampleTreatment {
    fn default() -> Self {
        Self {
            pan: 0.0,
            polarity_inverted: false,
            swap_stereo: false,
            normalize: false,
            remove_dc: false,
            resample: false,
            fade_stereo: false,
            start: 0.0,
            length: 1.0,
            trim: 0.0,
            declick: DeclickMode::OutOnly,
        }
    }
}

impl SampleTreatment {
    /// The amplitude under which the tail is cut, as a share of the
    /// sample's peak, or `None` when trim is off: -60 dB at the first
    /// step, the peak itself at the top.
    pub fn trim_threshold(&self) -> Option<f64> {
        let trim = self.trim.clamp(0.0, 1.0);
        (trim > 0.0).then(|| 10f64.powf(-60.0 * (1.0 - trim) / 20.0))
    }
}

/// FL's declicking modes: how a clip's edges are smoothed when the clip
/// has no fades of its own. Each is a pair of tiny fades, in and out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeclickMode {
    /// Only the end is smoothed; the attack is left as it is.
    #[default]
    OutOnly,
    /// A hair at the start, so a transient keeps its edge.
    Transient,
    Generic,
    Smooth,
    /// Longer at both ends, for clips that overlap.
    Crossfade,
    /// The end lets go slowly, as a sample ringing past its clip would.
    Bleeding,
}

impl DeclickMode {
    pub const ALL: [DeclickMode; 6] = [
        DeclickMode::OutOnly,
        DeclickMode::Transient,
        DeclickMode::Generic,
        DeclickMode::Smooth,
        DeclickMode::Crossfade,
        DeclickMode::Bleeding,
    ];

    /// The fades it stands for, in milliseconds: in, out.
    pub fn fades_ms(self) -> (f64, f64) {
        match self {
            DeclickMode::OutOnly => (0.0, 1.0),
            DeclickMode::Transient => (0.3, 2.0),
            DeclickMode::Generic => (2.0, 5.0),
            DeclickMode::Smooth => (10.0, 20.0),
            DeclickMode::Crossfade => (5.0, 5.0),
            DeclickMode::Bleeding => (0.0, 8.0),
        }
    }

    /// As FL names it.
    pub fn name(self) -> &'static str {
        match self {
            DeclickMode::OutOnly => "Out only",
            DeclickMode::Transient => "Transient",
            DeclickMode::Generic => "Generic",
            DeclickMode::Smooth => "Smooth",
            DeclickMode::Crossfade => "Crossfade",
            DeclickMode::Bleeding => "Bleeding",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|m| m.name().eq_ignore_ascii_case(name.trim()))
    }
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

impl AudioClip {
    /// How much longer than its file the sample lasts as it plays: the
    /// stretch, and in resample also the pitch, as a tape played faster is
    /// higher and shorter.
    pub fn time_factor(&self) -> f64 {
        let stretch = if self.stretch_ratio <= 0.01 {
            1.0
        } else {
            self.stretch_ratio
        };
        if self.treatment.resample {
            stretch / 2f64.powf(self.pitch_semitones / 12.0)
        } else {
            stretch
        }
    }
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

/// The lane a freeze render sits on.
///
/// Out of the way of the take lanes, which count up from zero, so a
/// frozen track's render never shows up as one more take to comp.
pub const FREEZE_LANE: u32 = u32::MAX;
