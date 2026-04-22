//! Streaming audio file reader — read-ahead ring buffer for gapless
//! playback from disk + a memory-mapped sample provider for the
//! sample library. Keeps the audio thread out of the disk I/O path.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::path::Path;

/// A block-based producer/consumer ring buffer — the disk I/O thread
/// calls `push_block` with the next chunk of samples; the audio
/// thread calls `pop_samples` to pull as many samples as the block
/// needs. `underruns` increments whenever the consumer requests more
/// than the buffer holds.
pub struct ReadAheadRing {
    queue: VecDeque<f32>,
    capacity: usize,
    underruns: u64,
}

impl ReadAheadRing {
    pub fn new(capacity_samples: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(capacity_samples.max(1)),
            capacity: capacity_samples.max(1),
            underruns: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.queue.len() >= self.capacity
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn underruns(&self) -> u64 {
        self.underruns
    }

    /// Push a block of samples. Samples that don't fit are dropped
    /// — call `available_capacity` first if you can't afford to drop.
    /// Returns how many samples were accepted.
    pub fn push_block(&mut self, samples: &[f32]) -> usize {
        let free = self.capacity - self.queue.len();
        let take = samples.len().min(free);
        self.queue.extend(samples[..take].iter().copied());
        take
    }

    pub fn available_capacity(&self) -> usize {
        self.capacity - self.queue.len()
    }

    /// Pop up to `dst.len()` samples into `dst`. Returns the number
    /// actually written; missing samples are zero-filled to keep the
    /// audio thread glitch-free on underrun.
    pub fn pop_samples(&mut self, dst: &mut [f32]) -> usize {
        let mut written = 0;
        for slot in dst.iter_mut() {
            if let Some(s) = self.queue.pop_front() {
                *slot = s;
                written += 1;
            } else {
                *slot = 0.0;
            }
        }
        if written < dst.len() {
            self.underruns += 1;
        }
        written
    }

    /// Fraction of the ring currently filled — `[0.0, 1.0]`. Useful
    /// for the disk thread to decide whether to sleep.
    pub fn fill_fraction(&self) -> f32 {
        self.queue.len() as f32 / self.capacity as f32
    }

    pub fn clear(&mut self) {
        self.queue.clear();
    }
}

/// Memory-mapped f32 sample provider — reads a raw little-endian
/// f32 file and exposes random access through `sample_at(index)` or
/// `read_range(start, len)`. Backed by `std::fs::File` + memmap-
/// free slice so the OS handles paging.
pub struct MappedF32File {
    samples: Vec<f32>,
    sample_rate: u32,
}

impl MappedF32File {
    /// Read a raw f32le file into memory. For real memory-mapping
    /// the runtime should plug in `memmap2::Mmap`; this fallback
    /// reads the whole file so the interface stays identical.
    pub fn open_raw(path: &Path, sample_rate: u32) -> io::Result<Self> {
        let mut file = std::fs::File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let samples: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        Ok(Self {
            samples,
            sample_rate,
        })
    }

    pub fn from_samples(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            samples,
            sample_rate,
        }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn sample_at(&self, index: usize) -> Option<f32> {
        self.samples.get(index).copied()
    }

    pub fn read_range(&self, start: usize, len: usize) -> &[f32] {
        let end = (start + len).min(self.samples.len());
        if start >= self.samples.len() {
            &[]
        } else {
            &self.samples[start..end]
        }
    }
}

/// Simple round-trip audio file cache keyed by path string. LRU-ish
/// via a `last_used_tick` counter on each entry. `max_bytes` bounds
/// total memory footprint.
pub struct AudioFileCache {
    entries: Vec<CacheEntry>,
    max_bytes: usize,
    tick: u64,
}

struct CacheEntry {
    key: String,
    samples: Vec<f32>,
    last_used_tick: u64,
}

impl AudioFileCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_bytes,
            tick: 0,
        }
    }

    pub fn get(&mut self, key: &str) -> Option<&[f32]> {
        self.tick += 1;
        let tick = self.tick;
        for entry in self.entries.iter_mut() {
            if entry.key == key {
                entry.last_used_tick = tick;
                return Some(&entry.samples);
            }
        }
        None
    }

    pub fn insert(&mut self, key: impl Into<String>, samples: Vec<f32>) {
        let tick = self.tick + 1;
        self.tick = tick;
        // Evict LRU entries until the insert fits.
        let incoming = samples.len() * std::mem::size_of::<f32>();
        while self.total_bytes() + incoming > self.max_bytes && !self.entries.is_empty() {
            let victim = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_used_tick)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.entries.remove(victim);
        }
        self.entries.push(CacheEntry {
            key: key.into(),
            samples,
            last_used_tick: tick,
        });
    }

    pub fn total_bytes(&self) -> usize {
        self.entries
            .iter()
            .map(|e| e.samples.len() * std::mem::size_of::<f32>())
            .sum()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_push_and_pop_in_order() {
        let mut ring = ReadAheadRing::new(16);
        ring.push_block(&[1.0, 2.0, 3.0, 4.0]);
        let mut buf = [0.0; 4];
        let written = ring.pop_samples(&mut buf);
        assert_eq!(written, 4);
        assert_eq!(buf, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn ring_drops_overflow_samples() {
        let mut ring = ReadAheadRing::new(4);
        let accepted = ring.push_block(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(accepted, 4);
        assert!(ring.is_full());
    }

    #[test]
    fn ring_underrun_increments_counter_and_zero_fills() {
        let mut ring = ReadAheadRing::new(8);
        ring.push_block(&[0.5, 0.5]);
        let mut buf = [99.0; 4];
        let written = ring.pop_samples(&mut buf);
        assert_eq!(written, 2);
        assert_eq!(buf, [0.5, 0.5, 0.0, 0.0]);
        assert_eq!(ring.underruns(), 1);
    }

    #[test]
    fn ring_fill_fraction_reports_correct_ratio() {
        let mut ring = ReadAheadRing::new(10);
        ring.push_block(&[0.0; 3]);
        assert!((ring.fill_fraction() - 0.3).abs() < 1e-3);
    }

    #[test]
    fn mapped_file_from_samples_random_access() {
        let f = MappedF32File::from_samples(vec![1.0, 2.0, 3.0], 48_000);
        assert_eq!(f.sample_rate(), 48_000);
        assert_eq!(f.len(), 3);
        assert_eq!(f.sample_at(1), Some(2.0));
        assert_eq!(f.sample_at(99), None);
        assert_eq!(f.read_range(1, 5), &[2.0, 3.0]);
        assert_eq!(f.read_range(99, 5), &[] as &[f32]);
    }

    #[test]
    fn cache_insert_hits_on_subsequent_get() {
        let mut cache = AudioFileCache::new(1024);
        cache.insert("one", vec![1.0, 2.0]);
        assert_eq!(cache.get("one"), Some(&[1.0, 2.0][..]));
        assert_eq!(cache.get("missing"), None);
    }

    #[test]
    fn cache_evicts_lru_when_full() {
        let mut cache = AudioFileCache::new(32); // room for 2 × 4-sample entries
        cache.insert("a", vec![0.0_f32; 4]);
        cache.insert("b", vec![0.0_f32; 4]);
        // Touch `a` so `b` is the LRU.
        let _ = cache.get("a");
        cache.insert("c", vec![0.0_f32; 4]); // should evict `b`.
        assert!(cache.get("a").is_some());
        assert!(cache.get("c").is_some());
        assert!(cache.get("b").is_none());
        assert_eq!(cache.len(), 2);
    }
}
