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
