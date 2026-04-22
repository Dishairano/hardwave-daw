//! Hardwave Project — project model, track system, serialization.

pub mod automation;
pub mod automation_clip;
pub mod automation_recording;
pub mod channel_rack;
pub mod clip;
pub mod lfo;
pub mod marketplace;
pub mod mixer;
pub mod project;
pub mod recording_session;
pub mod sidechain_routing;
pub mod tempo;
pub mod track;
pub mod track_freeze;

pub use automation::{AutomationLane, AutomationPoint, AutomationTarget};
pub use automation_recording::{AutomationRecorder, WriteMode};
pub use clip::{AudioClip, ClipId, ClipPlacement};
pub use lfo::{LfoRate, LfoShape};
pub use mixer::{ChannelStrip, Send as MixerSend};
pub use project::Project;
pub use tempo::{TempoEntry, TempoMap};
pub use track::{Track, TrackId, TrackKind};
