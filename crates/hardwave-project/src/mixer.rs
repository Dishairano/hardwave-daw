use serde::{Deserialize, Serialize};

use crate::track::TrackId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Send {
    pub target: TrackId,
    pub gain_db: f64,
    pub pre_fader: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelStrip {
    pub track_id: TrackId,
    pub input_gain_db: f64,
    pub fader_db: f64,
    pub pan: f64,
}
