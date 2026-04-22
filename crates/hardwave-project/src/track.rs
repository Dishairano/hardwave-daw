use serde::{Deserialize, Serialize};

use crate::automation::AutomationLane;
use crate::clip::ClipPlacement;

pub type TrackId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrackKind {
    Audio,
    Midi,
    Bus,
    Return,
    Master,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSlot {
    pub id: String,
    pub plugin_id: String,
    pub enabled: bool,
    /// Opaque plugin state (saved/restored by the host).
    #[serde(with = "serde_bytes_opt")]
    pub state: Option<Vec<u8>>,
    /// Sidechain source track, if any.
    pub sidechain_source: Option<TrackId>,
    /// Dry/wet mix for this slot in [0, 1]. 1.0 = fully processed, 0.0 = bypass audible effect.
    #[serde(default = "default_wet")]
    pub wet: f32,
}

fn default_wet() -> f32 {
    1.0
}

fn default_stereo_separation() -> f64 {
    1.0
}

fn default_monitor_input() -> bool {
    true
}

fn default_pitch_semitones() -> i32 {
    0
}

fn default_fine_tune_cents() -> f32 {
    0.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,
    pub color: String,

    // Audio routing
    pub volume_db: f64,
    pub pan: f64,
    pub muted: bool,
    pub soloed: bool,
    pub solo_safe: bool,
    pub armed: bool,
    pub output_bus: Option<TrackId>,
    /// When `armed`, route audio input through this track (through its FX
    /// chain) so the performer can hear themselves. Users can disable this
    /// to arm a track without monitoring (e.g. when monitoring via hardware).
    #[serde(default = "default_monitor_input")]
    pub monitor_input: bool,

    /// Flip sample polarity on this track. Applied before the fader.
    #[serde(default)]
    pub phase_invert: bool,
    /// Swap left and right channels on this track. Applied before the fader.
    #[serde(default)]
    pub swap_lr: bool,
    /// Stereo separation: 0.0 = mono (L=R), 1.0 = normal, >1.0 = widened.
    #[serde(default = "default_stereo_separation")]
    pub stereo_separation: f64,
    /// Positive value delays the track by N samples, negative advances it.
    /// Applied before the fader so metering reflects the shifted signal.
    #[serde(default)]
    pub delay_samples: i64,

    /// Coarse pitch offset in semitones, applied as a source resample factor
    /// multiplied with each audio clip's own pitch setting. Range clamped to
    /// -24..=24 by the Tauri command.
    #[serde(default = "default_pitch_semitones")]
    pub pitch_semitones: i32,
    /// Fine tune in cents, combined with pitch_semitones on the same resample
    /// factor. Range clamped to -100.0..=100.0 by the Tauri command.
    #[serde(default = "default_fine_tune_cents")]
    pub fine_tune_cents: f32,

    // Plugin chain
    pub inserts: Vec<PluginSlot>,

    // Sends
    pub sends: Vec<crate::mixer::Send>,

    // Clips on this track
    pub clips: Vec<ClipPlacement>,

    // Automation
    pub automation_lanes: Vec<AutomationLane>,
}

impl Track {
    pub fn new_audio(id: String, name: String) -> Self {
        Self {
            id,
            name,
            kind: TrackKind::Audio,
            color: "#7c3aed".into(),
            volume_db: 0.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            solo_safe: false,
            armed: false,
            output_bus: None,
            monitor_input: true,
            phase_invert: false,
            swap_lr: false,
            stereo_separation: 1.0,
            delay_samples: 0,
            pitch_semitones: 0,
            fine_tune_cents: 0.0,
            inserts: Vec::new(),
            sends: Vec::new(),
            clips: Vec::new(),
            automation_lanes: Vec::new(),
        }
    }

    pub fn new_midi(id: String, name: String) -> Self {
        let mut t = Self::new_audio(id, name);
        t.kind = TrackKind::Midi;
        t.color = "#06b6d4".into();
        t
    }

    pub fn new_bus(id: String, name: String) -> Self {
        let mut t = Self::new_audio(id, name);
        t.kind = TrackKind::Bus;
        t.color = "#22c55e".into();
        t
    }

    pub fn new_master(id: String) -> Self {
        let mut t = Self::new_audio(id, "Master".into());
        t.kind = TrackKind::Master;
        t.color = "#ef4444".into();
        t
    }
}

mod serde_bytes_opt {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(val: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match val {
            Some(bytes) => s.serialize_bytes(bytes),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        Option::<Vec<u8>>::deserialize(d)
    }
}
