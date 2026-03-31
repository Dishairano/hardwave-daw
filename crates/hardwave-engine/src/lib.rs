//! Hardwave Engine — real-time audio graph, transport, and orchestration.

pub mod graph;
pub mod transport;
pub mod engine;
pub mod audio_pool;
pub mod track_node;
pub mod master_node;

pub use engine::DawEngine;
pub use transport::{TransportState, TransportCommand};
pub use graph::{AudioGraph, AudioNode, NodeId, ProcessContext};
pub use audio_pool::{AudioPool, AudioBuffer};
pub use track_node::{TrackNode, ClipRegion};
pub use master_node::MasterNode;
