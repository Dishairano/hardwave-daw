//! AudioPool — shared store of decoded audio buffers, accessible from the audio thread.
//!
//! Audio data is loaded on a background thread and inserted via Arc. The audio thread
//! reads immutably — no locks in the hot path.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// A decoded audio file stored as deinterleaved f32 channels.
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: u32,
    pub num_frames: usize,
}

impl AudioBuffer {
    /// Get a sample from a specific channel and frame, or 0.0 if out of bounds.
    #[inline]
    pub fn sample(&self, channel: usize, frame: usize) -> f32 {
        self.channels
            .get(channel)
            .and_then(|ch| ch.get(frame))
            .copied()
            .unwrap_or(0.0)
    }
}

/// Pool of loaded audio buffers keyed by a unique source ID (typically the file path hash).
#[derive(Clone)]
pub struct AudioPool {
    buffers: Arc<RwLock<HashMap<String, Arc<AudioBuffer>>>>,
}

impl AudioPool {
    pub fn new() -> Self {
        Self {
            buffers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Insert a decoded audio buffer. Called from the loading thread.
    pub fn insert(&self, id: String, buffer: AudioBuffer) {
        self.buffers.write().insert(id, Arc::new(buffer));
    }

    /// Get a reference to a buffer. The Arc ensures the audio thread can hold it
    /// without blocking the loading thread.
    pub fn get(&self, id: &str) -> Option<Arc<AudioBuffer>> {
        self.buffers.read().get(id).cloned()
    }

    /// Remove a buffer by ID.
    pub fn remove(&self, id: &str) {
        self.buffers.write().remove(id);
    }

    /// Check if a buffer is loaded.
    pub fn contains(&self, id: &str) -> bool {
        self.buffers.read().contains_key(id)
    }
}

impl Default for AudioPool {
    fn default() -> Self {
        Self::new()
    }
}
