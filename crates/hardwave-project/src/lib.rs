//! Hardwave Project — project model, track system, serialization.

pub mod track;
pub mod clip;
pub mod automation;
pub mod tempo;
pub mod mixer;
pub mod project;

pub use project::Project;
pub use track::{Track, TrackId, TrackKind};
pub use clip::{AudioClip, ClipPlacement, ClipId};
pub use automation::{AutomationLane, AutomationPoint, AutomationTarget};
pub use tempo::{TempoMap, TempoEntry};
pub use mixer::{ChannelStrip, Send as MixerSend};
