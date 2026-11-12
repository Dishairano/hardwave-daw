//! Hardwave Engine — real-time audio graph, transport, and orchestration.

pub mod audio_pool;
pub mod audio_prefs;
pub mod automation_eval;
pub mod check_voice;
pub mod clip_edit;
pub mod engine;
pub mod graph;
pub mod input_node;
pub mod session;
pub mod source_paths;
pub use input_node::CaptureTap;
pub mod insert_chain;
pub mod link;
pub mod live_midi_fx;
pub mod master_node;
pub mod master_tap;
pub mod metronome;
pub mod midi_track_node;
pub mod pan;
pub mod parallel;

/// Re-exported so callers outside this crate can hold the same atomic
/// float the meters and gain-reduction channels use.
pub use atomic_float;
/// Which threads make the sound, and lowering background work (see the module).
pub use hardwave_audio_io::thread_priority;
pub mod preview_player;
pub mod tempo_follow;
pub mod test_tone;
pub mod track_node;
pub mod transport;

pub use audio_pool::{AudioBuffer, AudioPool};
pub use audio_prefs::AudioPrefs;
pub use engine::{AudioSourceLoader, DawEngine, OfflineRender};
pub use graph::{AudioGraph, AudioNode, NodeId, ProcessContext};
pub use master_node::MasterNode;
pub use master_tap::{MasterTap, SharedMasterTap};
pub use midi_track_node::{MidiNoteRegion, MidiTrackNode};
pub use track_node::{ClipRegion, TrackNode};
pub use transport::{TransportCommand, TransportState};
