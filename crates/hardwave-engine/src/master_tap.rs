//! Shared circular buffer of recent master-bus output samples, populated by
//! the audio thread and read by the UI for visualizations (oscilloscope,
//! spectrum, etc.). Samples are interleaved stereo (L,R,L,R…).
//!
//! The audio thread uses `try_lock` and silently drops the block if the UI
//! is currently snapshotting — occasional gaps in an oscilloscope are
//! imperceptible, but stalling the audio thread is not. parking_lot's
//! Mutex is fast enough that contention is rare in practice.

use parking_lot::Mutex;
use std::sync::Arc;

/// Default capacity: 8192 stereo frames, ~170 ms at 48 kHz. That's well
/// above any reasonable oscilloscope window (typically 5 – 50 ms).
pub const DEFAULT_CAPACITY_FRAMES: usize = 8192;

pub type SharedMasterTap = Arc<Mutex<MasterTap>>;

pub fn new_shared() -> SharedMasterTap {
    Arc::new(Mutex::new(MasterTap::new(DEFAULT_CAPACITY_FRAMES)))
}

pub struct MasterTap {
    /// Interleaved stereo samples. Length is `capacity_frames * 2`.
    samples: Vec<f32>,
    /// Next slot to write. Wraps at `samples.len()`.
    write: usize,
    /// Becomes true once the buffer has been fully populated — before that,
    /// only `write` samples are valid.
    filled: bool,
}

impl MasterTap {
    pub fn new(capacity_frames: usize) -> Self {
        let capacity_frames = capacity_frames.max(64);
        Self {
            samples: vec![0.0; capacity_frames * 2],
            write: 0,
            filled: false,
        }
    }

    /// Append a block of interleaved stereo samples. Silently wraps.
    pub fn push_block(&mut self, block: &[f32]) {
        if block.is_empty() || self.samples.is_empty() {
            return;
        }
        let cap = self.samples.len();
        for &s in block {
            self.samples[self.write] = s;
            self.write += 1;
            if self.write >= cap {
                self.write = 0;
                self.filled = true;
            }
        }
    }

    /// Copy the most recent `n_frames` stereo frames into a fresh Vec<f32>
    /// in interleaved order. Returns a shorter buffer if less than
    /// `n_frames` have been written yet.
    pub fn snapshot_interleaved(&self, n_frames: usize) -> Vec<f32> {
        let cap = self.samples.len();
        let wanted = (n_frames * 2).min(cap);
        let available = if self.filled { cap } else { self.write };
        let wanted = wanted.min(available);
        if wanted == 0 {
            return Vec::new();
        }
        let mut out = vec![0.0; wanted];
        // Range to copy is the `wanted` samples ending at index `self.write`.
        let start = (self.write + cap - wanted) % cap;
        if start + wanted <= cap {
            out.copy_from_slice(&self.samples[start..start + wanted]);
        } else {
            let first = cap - start;
            out[..first].copy_from_slice(&self.samples[start..cap]);
            out[first..].copy_from_slice(&self.samples[..wanted - first]);
        }
        out
    }

    pub fn capacity_frames(&self) -> usize {
        self.samples.len() / 2
    }

    pub fn reset(&mut self) {
        for s in self.samples.iter_mut() {
            *s = 0.0;
        }
        self.write = 0;
        self.filled = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_empty() {
        let tap = MasterTap::new(128);
        assert!(tap.snapshot_interleaved(32).is_empty());
    }

    #[test]
    fn snapshot_partial() {
        let mut tap = MasterTap::new(128);
        // 5 stereo frames (10 interleaved samples) written.
        let block: Vec<f32> = (0..10).map(|i| i as f32).collect();
        tap.push_block(&block);
        let snap = tap.snapshot_interleaved(5);
        assert_eq!(snap.len(), 10);
        assert_eq!(snap, block);
    }

    #[test]
    fn snapshot_wraps() {
        let mut tap = MasterTap::new(64);
        let total_samples = 64 * 2 + 20;
        let block: Vec<f32> = (0..total_samples).map(|i| i as f32).collect();
        tap.push_block(&block);
        let last32 = tap.snapshot_interleaved(16);
        assert_eq!(last32.len(), 32);
        let expected: Vec<f32> = (total_samples - 32..total_samples)
            .map(|i| i as f32)
            .collect();
        assert_eq!(last32, expected);
    }

    #[test]
    fn snapshot_truncates_to_capacity() {
        let mut tap = MasterTap::new(32);
        let block: Vec<f32> = (0..64).map(|i| i as f32).collect();
        tap.push_block(&block);
        // Ask for more than capacity — should clamp.
        let snap = tap.snapshot_interleaved(1000);
        assert_eq!(snap.len(), 64);
    }
}
