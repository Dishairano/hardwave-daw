//! AudioPool — shared store of decoded audio buffers, accessible from the audio thread.
//!
//! Audio data is loaded on a background thread and inserted via Arc. The audio thread
//! reads immutably — no locks in the hot path. A byte-size cap with FIFO eviction
//! keeps memory bounded across long sessions with many samples loaded.

use parking_lot::RwLock;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Default cap on decoded audio memory before FIFO eviction kicks in. 2 GiB is
/// high enough to never bite most projects but low enough to prevent runaway
/// use across long sessions with a lot of sampler content.
pub const DEFAULT_MAX_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

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

    /// Size of the decoded audio in bytes (channels × frames × 4 bytes/sample).
    pub fn bytes(&self) -> u64 {
        let mut total: u64 = 0;
        for ch in &self.channels {
            total = total.saturating_add((ch.len() as u64).saturating_mul(4));
        }
        total
    }
}

struct PoolInner {
    buffers: HashMap<String, Arc<AudioBuffer>>,
    order: VecDeque<String>,
    bytes_used: u64,
}

impl PoolInner {
    fn new() -> Self {
        Self {
            buffers: HashMap::new(),
            order: VecDeque::new(),
            bytes_used: 0,
        }
    }
}

/// Snapshot of pool usage, safe to return across threads.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct AudioCacheStats {
    #[serde(rename = "bytesUsed")]
    pub bytes_used: u64,
    #[serde(rename = "maxBytes")]
    pub max_bytes: u64,
    #[serde(rename = "entryCount")]
    pub entry_count: u64,
}

/// Pool of loaded audio buffers keyed by a unique source ID (typically the file path hash).
#[derive(Clone)]
pub struct AudioPool {
    inner: Arc<RwLock<PoolInner>>,
    max_bytes: Arc<AtomicU64>,
}

impl AudioPool {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(PoolInner::new())),
            max_bytes: Arc::new(AtomicU64::new(DEFAULT_MAX_CACHE_BYTES)),
        }
    }

    /// Insert a decoded audio buffer. FIFO-evicts oldest entries first if
    /// the new total would exceed `max_bytes`. Never evicts the entry being
    /// inserted — if the buffer itself is larger than the cap the cap is
    /// temporarily exceeded rather than failing the insert.
    pub fn insert(&self, id: String, buffer: AudioBuffer) {
        let new_bytes = buffer.bytes();
        let cap = self.max_bytes.load(Ordering::Relaxed);
        let mut inner = self.inner.write();

        // If replacing an existing entry, subtract its bytes first.
        if let Some(existing) = inner.buffers.get(&id) {
            inner.bytes_used = inner.bytes_used.saturating_sub(existing.bytes());
            // Move the id to the back of the order queue below; drop old now.
            if let Some(pos) = inner.order.iter().position(|x| x == &id) {
                inner.order.remove(pos);
            }
        }

        // FIFO-evict until the new entry fits.
        while inner.bytes_used.saturating_add(new_bytes) > cap && !inner.order.is_empty() {
            if let Some(oldest_id) = inner.order.pop_front() {
                if let Some(old) = inner.buffers.remove(&oldest_id) {
                    inner.bytes_used = inner.bytes_used.saturating_sub(old.bytes());
                }
            } else {
                break;
            }
        }

        inner.buffers.insert(id.clone(), Arc::new(buffer));
        inner.order.push_back(id);
        inner.bytes_used = inner.bytes_used.saturating_add(new_bytes);
    }

    /// Get a reference to a buffer. The Arc ensures the audio thread can hold it
    /// without blocking the loading thread.
    pub fn get(&self, id: &str) -> Option<Arc<AudioBuffer>> {
        self.inner.read().buffers.get(id).cloned()
    }

    /// Remove a buffer by ID.
    pub fn remove(&self, id: &str) {
        let mut inner = self.inner.write();
        if let Some(old) = inner.buffers.remove(id) {
            inner.bytes_used = inner.bytes_used.saturating_sub(old.bytes());
        }
        if let Some(pos) = inner.order.iter().position(|x| x == id) {
            inner.order.remove(pos);
        }
    }

    /// Check if a buffer is loaded.
    pub fn contains(&self, id: &str) -> bool {
        self.inner.read().buffers.contains_key(id)
    }

    /// Snapshot of current byte usage, cap, and entry count.
    pub fn stats(&self) -> AudioCacheStats {
        let inner = self.inner.read();
        AudioCacheStats {
            bytes_used: inner.bytes_used,
            max_bytes: self.max_bytes.load(Ordering::Relaxed),
            entry_count: inner.buffers.len() as u64,
        }
    }

    /// Re-resample every cached buffer whose native sample rate differs from
    /// `dst_sr` using the supplied closure (we don't import a DSP dep here —
    /// the caller wires in `hardwave_dsp::resample_channels`). Returns the
    /// number of buffers that were rewritten.
    ///
    /// Called by the engine when the audio device's sample rate changes
    /// (user picks a new device / cpal falls back on stream open). Without
    /// this the pool stays at the OLD rate while cpal pulls at the new one,
    /// playing every clip back at the wrong pitch.
    pub fn resample_all<F>(&self, dst_sr: u32, mut do_resample: F) -> usize
    where
        F: FnMut(&[Vec<f32>], u32, u32) -> Option<Vec<Vec<f32>>>,
    {
        let mut count = 0_usize;
        let mut inner = self.inner.write();
        let ids: Vec<String> = inner.buffers.keys().cloned().collect();
        for id in ids {
            let buf = match inner.buffers.get(&id) {
                Some(b) => Arc::clone(b),
                None => continue,
            };
            if buf.sample_rate == dst_sr || buf.num_frames == 0 {
                continue;
            }
            let Some(new_channels) = do_resample(&buf.channels, buf.sample_rate, dst_sr) else {
                continue;
            };
            let new_frames = new_channels.first().map(|c| c.len()).unwrap_or(0);
            let new_buf = AudioBuffer {
                channels: new_channels,
                sample_rate: dst_sr,
                num_frames: new_frames,
            };
            let old_bytes = buf.bytes();
            let new_bytes = new_buf.bytes();
            inner.bytes_used = inner
                .bytes_used
                .saturating_sub(old_bytes)
                .saturating_add(new_bytes);
            inner.buffers.insert(id, Arc::new(new_buf));
            count += 1;
        }
        count
    }

    /// Update the cache cap. Immediately evicts oldest entries if the new
    /// cap is lower than current usage.
    pub fn set_max_bytes(&self, max_bytes: u64) {
        self.max_bytes.store(max_bytes, Ordering::Relaxed);
        let mut inner = self.inner.write();
        while inner.bytes_used > max_bytes {
            if let Some(oldest_id) = inner.order.pop_front() {
                if let Some(old) = inner.buffers.remove(&oldest_id) {
                    inner.bytes_used = inner.bytes_used.saturating_sub(old.bytes());
                }
            } else {
                break;
            }
        }
    }
}

impl Default for AudioPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(secs: u64) -> AudioBuffer {
        let frames = (secs * 48_000) as usize;
        AudioBuffer {
            channels: vec![vec![0.0; frames]; 2],
            sample_rate: 48_000,
            num_frames: frames,
        }
    }

    #[test]
    fn stats_track_bytes_and_count() {
        let pool = AudioPool::new();
        assert_eq!(pool.stats().entry_count, 0);
        assert_eq!(pool.stats().bytes_used, 0);
        pool.insert("a".into(), buf(1));
        let s = pool.stats();
        assert_eq!(s.entry_count, 1);
        // 1 second * 48000 frames * 2 channels * 4 bytes = 384000
        assert_eq!(s.bytes_used, 384_000);
    }

    #[test]
    fn remove_releases_bytes() {
        let pool = AudioPool::new();
        pool.insert("a".into(), buf(1));
        pool.insert("b".into(), buf(2));
        assert_eq!(pool.stats().entry_count, 2);
        pool.remove("a");
        assert_eq!(pool.stats().entry_count, 1);
        assert_eq!(pool.stats().bytes_used, 2 * 384_000);
    }

    #[test]
    fn fifo_eviction_when_over_cap() {
        let pool = AudioPool::new();
        pool.set_max_bytes(500_000);
        pool.insert("a".into(), buf(1)); // 384000 bytes
        assert!(pool.contains("a"));
        // Second insert would push us over 500k; oldest ("a") must be evicted.
        pool.insert("b".into(), buf(1));
        assert!(!pool.contains("a"));
        assert!(pool.contains("b"));
    }

    #[test]
    fn shrinking_cap_evicts_immediately() {
        let pool = AudioPool::new();
        pool.insert("a".into(), buf(1));
        pool.insert("b".into(), buf(1));
        pool.insert("c".into(), buf(1));
        assert_eq!(pool.stats().entry_count, 3);
        // Shrink cap below the current total — FIFO eviction (a then b).
        pool.set_max_bytes(400_000);
        assert!(!pool.contains("a"));
        assert!(!pool.contains("b"));
        assert!(pool.contains("c"));
    }

    #[test]
    fn resample_all_rewrites_only_mismatched_buffers() {
        let pool = AudioPool::new();
        // 48k buffer that needs to drop to 44.1k.
        pool.insert(
            "a".into(),
            AudioBuffer {
                channels: vec![vec![1.0_f32; 4800]; 2],
                sample_rate: 48_000,
                num_frames: 4800,
            },
        );
        // Already at the target rate — should be skipped.
        pool.insert(
            "b".into(),
            AudioBuffer {
                channels: vec![vec![0.5_f32; 441]; 2],
                sample_rate: 44_100,
                num_frames: 441,
            },
        );
        let mut calls = 0;
        let n = pool.resample_all(44_100, |chs, src, dst| {
            calls += 1;
            assert_eq!(src, 48_000);
            assert_eq!(dst, 44_100);
            // Pretend rubato gave us a 44.1k buffer of half the frames.
            Some(
                chs.iter()
                    .map(|c| vec![1.0_f32; c.len() * 441 / 480])
                    .collect(),
            )
        });
        assert_eq!(n, 1);
        assert_eq!(calls, 1);
        let a = pool.get("a").unwrap();
        assert_eq!(a.sample_rate, 44_100);
        let b = pool.get("b").unwrap();
        assert_eq!(b.sample_rate, 44_100);
        assert_eq!(b.num_frames, 441); // untouched
    }

    #[test]
    fn replacing_same_id_does_not_evict() {
        let pool = AudioPool::new();
        pool.set_max_bytes(500_000);
        pool.insert("a".into(), buf(1));
        pool.insert("a".into(), buf(1));
        assert!(pool.contains("a"));
        assert_eq!(pool.stats().entry_count, 1);
    }
}
