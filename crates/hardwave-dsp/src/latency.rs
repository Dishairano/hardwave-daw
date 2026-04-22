//! Audio round-trip latency measurement + per-device offset storage.
//! The engine plays a short impulse through the audio output, captures
//! the input signal, and cross-correlates input against output to find
//! the offset in samples. That offset becomes the latency compensation
//! applied to recorded clips so they align with the playhead.

/// A single cross-correlation slot — the captured input samples + the
/// known impulse signal. Call `find_peak_offset` after both buffers
/// are populated to get the measured round-trip latency in samples.
pub struct LatencyMeasurement {
    impulse: Vec<f32>,
    captured: Vec<f32>,
    sample_rate: u32,
}

impl LatencyMeasurement {
    pub fn new(sample_rate: u32) -> Self {
        // Default impulse — a single spike. Callers can replace with
        // a noise burst or chirp for better SNR in noisy rooms.
        Self {
            impulse: vec![1.0],
            captured: Vec::new(),
            sample_rate,
        }
    }

    /// Replace the default impulse with a custom test signal. Useful
    /// for noise-burst or chirp-based measurements that tolerate
    /// ambient noise better than a single spike.
    pub fn set_impulse(&mut self, impulse: Vec<f32>) {
        self.impulse = impulse;
    }

    /// Push one captured input sample.
    pub fn push_capture(&mut self, sample: f32) {
        self.captured.push(sample);
    }

    /// Return the captured buffer length.
    pub fn capture_len(&self) -> usize {
        self.captured.len()
    }

    pub fn reset(&mut self) {
        self.captured.clear();
    }

    /// Find the offset in samples where the impulse best matches the
    /// captured signal. Returns `None` if the correlation is too
    /// weak (e.g. no signal reached the input). Uses naive O(N·M)
    /// cross-correlation — fine for small impulses (a few samples)
    /// and short capture windows (a few thousand samples).
    pub fn find_peak_offset(&self) -> Option<usize> {
        if self.captured.is_empty() || self.impulse.is_empty() {
            return None;
        }
        let n_capture = self.captured.len();
        let n_impulse = self.impulse.len();
        if n_capture < n_impulse {
            return None;
        }
        let mut best_offset = 0;
        let mut best_score = 0.0_f32;
        for offset in 0..=(n_capture - n_impulse) {
            let mut score = 0.0_f32;
            for i in 0..n_impulse {
                score += self.captured[offset + i] * self.impulse[i];
            }
            if score.abs() > best_score {
                best_score = score.abs();
                best_offset = offset;
            }
        }
        // Require the correlation to be above a noise-floor threshold.
        // Signal sum times some epsilon catches pure-silence inputs.
        let impulse_energy: f32 = self.impulse.iter().map(|s| s * s).sum();
        let threshold = impulse_energy * 0.1;
        if best_score >= threshold {
            Some(best_offset)
        } else {
            None
        }
    }

    /// Convert a sample offset into milliseconds at the session's
    /// sample rate.
    pub fn offset_to_ms(&self, samples: usize) -> f32 {
        if self.sample_rate == 0 {
            0.0
        } else {
            samples as f32 * 1000.0 / self.sample_rate as f32
        }
    }
}

/// Per-device latency offset storage — a plain map from device name
/// to compensation amount in samples. Persisting this to prefs is
/// the caller's job; this struct is just the in-memory representation.
#[derive(Default, Clone)]
pub struct DeviceLatencyOffsets {
    offsets: std::collections::HashMap<String, i32>,
}

impl DeviceLatencyOffsets {
    pub fn set(&mut self, device_name: impl Into<String>, offset_samples: i32) {
        self.offsets.insert(device_name.into(), offset_samples);
    }

    pub fn get(&self, device_name: &str) -> Option<i32> {
        self.offsets.get(device_name).copied()
    }

    pub fn remove(&mut self, device_name: &str) {
        self.offsets.remove(device_name);
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &i32)> {
        self.offsets.iter()
    }
}

/// Apply the latency offset to a recorded clip's position so it
/// aligns with the engine's playhead. Positive offsets shift the
/// clip earlier in time by that many samples (the classic "recording
/// lag compensation" adjustment). The clip's raw start sample is
/// what the engine saw at record-start; the adjusted position is
/// what gets placed on the arrangement.
pub fn apply_offset(raw_start_sample: u64, offset_samples: i32) -> u64 {
    if offset_samples >= 0 {
        raw_start_sample.saturating_sub(offset_samples as u64)
    } else {
        raw_start_sample.saturating_add((-offset_samples) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_peak_offset_detects_impulse_arrival() {
        let mut m = LatencyMeasurement::new(48_000);
        // Default impulse = [1.0]. Capture has the impulse at offset 500.
        let mut capture = vec![0.0; 2000];
        capture[500] = 1.0;
        for &s in &capture {
            m.push_capture(s);
        }
        assert_eq!(m.find_peak_offset(), Some(500));
    }

    #[test]
    fn find_peak_offset_returns_none_for_silent_capture() {
        let mut m = LatencyMeasurement::new(48_000);
        for _ in 0..2000 {
            m.push_capture(0.0);
        }
        assert_eq!(m.find_peak_offset(), None);
    }

    #[test]
    fn find_peak_offset_handles_multi_sample_impulse() {
        let mut m = LatencyMeasurement::new(48_000);
        m.set_impulse(vec![0.5, 1.0, 0.5]);
        let mut capture = vec![0.0; 2000];
        // Place the impulse pattern at offset 800.
        capture[800] = 0.5;
        capture[801] = 1.0;
        capture[802] = 0.5;
        for &s in &capture {
            m.push_capture(s);
        }
        assert_eq!(m.find_peak_offset(), Some(800));
    }

    #[test]
    fn offset_to_ms_converts_sample_count_to_time() {
        let m = LatencyMeasurement::new(48_000);
        // 480 samples @ 48 kHz = 10 ms.
        let ms = m.offset_to_ms(480);
        assert!((ms - 10.0).abs() < 0.01);
    }

    #[test]
    fn device_offsets_persist_per_device_name() {
        let mut offsets = DeviceLatencyOffsets::default();
        offsets.set("Focusrite Scarlett 2i2", 240);
        offsets.set("MacBook Pro Microphone", 128);
        assert_eq!(offsets.len(), 2);
        assert_eq!(offsets.get("Focusrite Scarlett 2i2"), Some(240));
        assert_eq!(offsets.get("MacBook Pro Microphone"), Some(128));
        assert_eq!(offsets.get("Unknown Device"), None);
        offsets.remove("Focusrite Scarlett 2i2");
        assert_eq!(offsets.get("Focusrite Scarlett 2i2"), None);
    }

    #[test]
    fn apply_offset_shifts_clip_earlier_for_positive() {
        // Recorded at sample 10_000 with a 480-sample latency → the
        // actual sound was at sample 9_520.
        assert_eq!(apply_offset(10_000, 480), 9_520);
    }

    #[test]
    fn apply_offset_handles_negative_offset() {
        // Unusual but supported: negative offset shifts clip later
        // in time (for audio systems with negative perceived
        // latency e.g. monitoring with early return paths).
        assert_eq!(apply_offset(10_000, -240), 10_240);
    }

    #[test]
    fn apply_offset_saturates_at_zero() {
        // Latency larger than raw start sample — saturating_sub
        // clamps to 0 rather than panicking.
        assert_eq!(apply_offset(100, 500), 0);
    }

    #[test]
    fn reset_clears_captured_samples() {
        let mut m = LatencyMeasurement::new(48_000);
        for _ in 0..100 {
            m.push_capture(1.0);
        }
        assert_eq!(m.capture_len(), 100);
        m.reset();
        assert_eq!(m.capture_len(), 0);
    }
}
