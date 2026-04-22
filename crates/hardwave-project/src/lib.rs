//! Hardwave Project — project model, track system, serialization.

pub mod automation;
pub mod clip;
pub mod lfo;
pub mod mixer;
pub mod project;
pub mod tempo;
pub mod track;

pub use automation::{AutomationLane, AutomationPoint, AutomationTarget};
pub use clip::{AudioClip, ClipId, ClipPlacement};
pub use lfo::{LfoRate, LfoShape};
pub use mixer::{ChannelStrip, Send as MixerSend};
pub use project::Project;
pub use tempo::{TempoEntry, TempoMap};
pub use track::{Track, TrackId, TrackKind};
