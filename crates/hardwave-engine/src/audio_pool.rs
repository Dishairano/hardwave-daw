//! AudioPool — shared store of decoded audio buffers, accessible from the audio thread.
//!
//! Audio data is loaded on a background thread and inserted via Arc. The audio thread
//! reads immutably — no locks in the hot path. A byte-size cap with FIFO eviction
//! keeps memory bounded across long sessions with many samples loaded.

use hardwave_dsp::waveform_colour::WaveformColours;
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

/// A buffer's levels, for normalising it and taking a DC offset off: each
/// channel's average, and the loudest peak with and without that average.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelStats {
    pub peak: f32,
    pub peak_without_dc: f32,
    pub dc: [f32; 2],
}

impl AudioBuffer {
    /// The frame after the last one louder than `threshold` on any channel;
    /// 0 when nothing is.
    pub fn trim_end(&self, threshold: f32) -> u64 {
        let mut end = 0usize;
        for ch in &self.channels {
            if let Some(last) = ch.iter().rposition(|v| v.abs() > threshold) {
                end = end.max(last + 1);
            }
        }
        end as u64
    }

    /// Its levels. Reads the whole buffer twice; the pool keeps the answer.
    pub fn level_stats(&self) -> LevelStats {
        let mut dc = [0.0f32; 2];
        for (c, ch) in self.channels.iter().take(2).enumerate() {
            if !ch.is_empty() {
                dc[c] = (ch.iter().map(|v| *v as f64).sum::<f64>() / ch.len() as f64) as f32;
            }
        }
        if self.channels.len() == 1 {
            dc[1] = dc[0];
        }
        let (mut peak, mut peak_without_dc) = (0.0f32, 0.0f32);
        for (c, ch) in self.channels.iter().take(2).enumerate() {
            for v in ch {
                peak = peak.max(v.abs());
                peak_without_dc = peak_without_dc.max((v - dc[c]).abs());
            }
        }
        LevelStats {
            peak,
            peak_without_dc,
            dc,
        }
    }
}

struct PoolInner {
    buffers: HashMap<String, Arc<AudioBuffer>>,
    order: VecDeque<String>,
    bytes_used: u64,
    /// Each buffer's levels, worked out once, with the buffer they were
    /// worked out from (by address): a buffer replaced under the same id
    /// gets its own.
    levels: HashMap<String, (usize, LevelStats)>,
    /// Where each buffer's tail falls under a threshold (the bits of the
    /// threshold as the key), with the buffer's address, as `levels`.
    trims: HashMap<(String, u32), (usize, u64)>,
    /// Each buffer's waveform colours, with its address, as `levels`.
    colours: HashMap<String, (usize, Arc<WaveformColours>)>,
}

impl PoolInner {
    fn new() -> Self {
        Self {
            buffers: HashMap::new(),
            order: VecDeque::new(),
            bytes_used: 0,
            levels: HashMap::new(),
            trims: HashMap::new(),
            colours: HashMap::new(),
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
                inner.colours.remove(&oldest_id);
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

    /// A buffer's levels (`LevelStats`), worked out the first time they are
    /// asked for. Not for the audio thread: the first ask reads the buffer.
    pub fn level_stats(&self, id: &str) -> Option<LevelStats> {
        let buffer = self.get(id)?;
        let address = Arc::as_ptr(&buffer) as usize;
        if let Some((at, stats)) = self.inner.read().levels.get(id) {
            if *at == address {
                return Some(*stats);
            }
        }
        let stats = buffer.level_stats();
        self.inner
            .write()
            .levels
            .insert(id.to_string(), (address, stats));
        Some(stats)
    }

    /// The frame after the last one louder than `threshold` (a share of
    /// full scale) on any channel: where the sample's tail is cut. Worked
    /// out the first time it is asked for. Not for the audio thread.
    pub fn trim_end(&self, id: &str, threshold: f32) -> Option<u64> {
        let buffer = self.get(id)?;
        let address = Arc::as_ptr(&buffer) as usize;
        let key = (id.to_string(), threshold.to_bits());
        if let Some((at, end)) = self.inner.read().trims.get(&key) {
            if *at == address {
                return Some(*end);
            }
        }
        let end = buffer.trim_end(threshold);
        self.inner.write().trims.insert(key, (address, end));
        Some(end)
    }

    /// A buffer's waveform colours (FL Studio's Multiband colouring, see
    /// `hardwave_dsp::waveform_colour`), worked out the first time they are
    /// asked for. Not for the audio thread: the first ask reads the buffer.
    pub fn waveform_colours(&self, id: &str) -> Option<Arc<WaveformColours>> {
        let buffer = self.get(id)?;
        let address = Arc::as_ptr(&buffer) as usize;
        if let Some((at, colours)) = self.inner.read().colours.get(id) {
            if *at == address {
                return Some(colours.clone());
            }
        }
        let colours = Arc::new(WaveformColours::analyse(
            &buffer.channels,
            buffer.sample_rate,
        ));
        self.inner
            .write()
            .colours
            .insert(id.to_string(), (address, colours.clone()));
        Some(colours)
    }

    /// Remove a buffer by ID.
    pub fn remove(&self, id: &str) {
        let mut inner = self.inner.write();
        inner.levels.remove(id);
        inner.colours.remove(id);
        inner.trims.retain(|(k, _), _| k != id);
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
                inner.colours.remove(&oldest_id);
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

    #[test]
    fn levels_are_worked_out_once_per_buffer() {
        let pool = AudioPool::new();
        pool.insert(
            "a".into(),
            AudioBuffer {
                channels: vec![vec![0.5, 0.7, 0.3], vec![-0.2, -0.2, -0.2]],
                sample_rate: 48_000,
                num_frames: 3,
            },
        );
        let s = pool.level_stats("a").unwrap();
        assert!((s.dc[0] - 0.5).abs() < 1e-6 && (s.dc[1] + 0.2).abs() < 1e-6);
        assert!((s.peak - 0.7).abs() < 1e-6);
        assert!((s.peak_without_dc - 0.2).abs() < 1e-6);
        // Replaced under the same id: its own levels, not the old ones.
        pool.insert(
            "a".into(),
            AudioBuffer {
                channels: vec![vec![0.1, -0.1]],
                sample_rate: 48_000,
                num_frames: 2,
            },
        );
        let s = pool.level_stats("a").unwrap();
        assert_eq!(s.dc, [0.0, 0.0]);
        assert!((s.peak - 0.1).abs() < 1e-6);
        assert!(pool.level_stats("missing").is_none());
    }

    #[test]
    fn waveform_colours_are_worked_out_once_per_buffer() {
        let pool = AudioPool::new();
        let tone = |hz: f32| -> Vec<f32> {
            (0..4800)
                .map(|i| (std::f32::consts::TAU * hz * i as f32 / 48_000.0).sin() * 0.5)
                .collect()
        };
        let insert = |hz: f32| {
            pool.insert(
                "a".into(),
                AudioBuffer {
                    channels: vec![tone(hz)],
                    sample_rate: 48_000,
                    num_frames: 4800,
                },
            )
        };
        insert(50.0);
        let first = pool.waveform_colours("a").unwrap();
        assert!(Arc::ptr_eq(&first, &pool.waveform_colours("a").unwrap()));
        let red = first.colour(1000, 3800).unwrap();
        // Replaced under the same id: its own colours, not the old ones.
        insert(8000.0);
        let blue = pool
            .waveform_colours("a")
            .unwrap()
            .colour(1000, 3800)
            .unwrap();
        assert!(
            red[0] > red[2] && blue[2] > blue[0],
            "{red:?} then {blue:?}"
        );
        pool.remove("a");
        assert!(pool.waveform_colours("a").is_none());
    }

    #[test]
    fn the_tail_is_cut_where_it_falls_under_the_threshold() {
        let mut left = vec![0.0f32; 100];
        left[10] = 0.9;
        left[50] = 0.05;
        let mut right = vec![0.0f32; 100];
        right[70] = 0.2;
        let b = AudioBuffer {
            channels: vec![left, right],
            sample_rate: 48_000,
            num_frames: 100,
        };
        assert_eq!(b.trim_end(0.5), 11, "only the peak is louder than half");
        assert_eq!(b.trim_end(0.1), 71, "the right channel's blip counts");
        assert_eq!(b.trim_end(0.01), 71);
        assert_eq!(b.trim_end(0.95), 0, "nothing over the top: nothing left");
        let pool = AudioPool::new();
        pool.insert("b".into(), b);
        assert_eq!(pool.trim_end("b", 0.5), Some(11));
        assert_eq!(
            pool.trim_end("b", 0.5),
            Some(11),
            "the second ask is the cached one"
        );
        assert_eq!(pool.trim_end("nope", 0.5), None);
    }
}
