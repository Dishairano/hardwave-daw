//! Hardwave Engine — real-time audio graph, transport, and orchestration.

pub mod audio_pool;
pub mod engine;
pub mod graph;
pub mod master_node;
pub mod track_node;
pub mod transport;

pub use audio_pool::{AudioBuffer, AudioPool};
pub use engine::DawEngine;
pub use graph::{AudioGraph, AudioNode, NodeId, ProcessContext};
pub use master_node::MasterNode;
pub use track_node::{ClipRegion, TrackNode};
pub use transport::{TransportCommand, TransportState};
