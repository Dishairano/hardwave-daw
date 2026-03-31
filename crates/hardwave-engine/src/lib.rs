//! Hardwave Engine — real-time audio graph, transport, and orchestration.

pub mod graph;
pub mod transport;
pub mod engine;

pub use engine::DawEngine;
pub use transport::{TransportState, TransportCommand};
pub use graph::{AudioGraph, AudioNode, NodeId, ProcessContext};
