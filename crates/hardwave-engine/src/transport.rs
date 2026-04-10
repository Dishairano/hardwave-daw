//! Transport state — play/stop/record/loop, shared between audio thread and UI.

use atomic_float::AtomicF64;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Transport state shared between the audio thread and the UI.
/// All fields are atomic — no locks needed on the audio thread.
#[derive(Clone)]
pub struct TransportState {
    pub playing: Arc<AtomicBool>,
    pub recording: Arc<AtomicBool>,
    pub looping: Arc<AtomicBool>,

    /// Current playhead position in samples.
    pub position_samples: Arc<AtomicU64>,

    /// BPM (may change via automation).
    pub bpm: Arc<AtomicF64>,

    /// Loop start/end in samples.
    pub loop_start: Arc<AtomicU64>,
    pub loop_end: Arc<AtomicU64>,

    /// Sample rate.
    pub sample_rate: Arc<AtomicU64>,
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            playing: Arc::new(AtomicBool::new(false)),
            recording: Arc::new(AtomicBool::new(false)),
            looping: Arc::new(AtomicBool::new(false)),
            position_samples: Arc::new(AtomicU64::new(0)),
            bpm: Arc::new(AtomicF64::new(140.0)),
            loop_start: Arc::new(AtomicU64::new(0)),
            loop_end: Arc::new(AtomicU64::new(0)),
            sample_rate: Arc::new(AtomicU64::new(48000)),
        }
    }
}

impl TransportState {
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn position(&self) -> u64 {
        self.position_samples.load(Ordering::Relaxed)
    }

    pub fn advance(&self, frames: u64) {
        let pos = self.position_samples.fetch_add(frames, Ordering::Relaxed) + frames;

        // Loop handling
        if self.looping.load(Ordering::Relaxed) {
            let loop_end = self.loop_end.load(Ordering::Relaxed);
            if loop_end > 0 && pos >= loop_end {
                let loop_start = self.loop_start.load(Ordering::Relaxed);
                self.position_samples.store(loop_start, Ordering::Relaxed);
            }
        }
    }

    pub fn set_position(&self, pos: u64) {
        self.position_samples.store(pos, Ordering::Relaxed);
    }
}

/// Commands sent from UI thread to engine.
#[derive(Debug, Clone)]
pub enum TransportCommand {
    Play,
    Stop,
    Record,
    SetPosition(u64),
    SetBpm(f64),
    SetLoop(u64, u64),
    ToggleLoop,
}
