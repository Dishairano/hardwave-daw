//! Stereo delay line primitive with feedback, optional ping-pong,
//! and tempo-synced timing. Allocation-free on the hot path — the
//! ring buffers are sized up-front based on the max supported delay.
//!
//! This is the DSP core the eventual Delay plugin will wrap in a
//! parameter/UI layer.

use crate::biquad::Biquad;

/// Max delay in samples — 5 seconds at 48 kHz. Larger than any
/// realistic tempo-synced value and big enough for long feedback
/// tails.
pub const MAX_DELAY_SAMPLES: usize = 48_000 * 5;

/// Stereo delay line with optional per-channel ping-pong and a
/// feedback path that can run through an optional biquad filter
/// (low-pass or high-pass for shaping the tail).
pub struct StereoDelayLine {
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    write_pos: usize,
    delay_samples_l: usize,
    delay_samples_r: usize,
    feedback: f32,
    ping_pong: bool,
    filter_l: Option<Biquad>,
    filter_r: Option<Biquad>,
}

impl Default for StereoDelayLine {
    fn default() -> Self {
        Self::new(MAX_DELAY_SAMPLES)
    }
}

impl StereoDelayLine {
    /// Allocate a delay line with the given capacity. Capacity is
    /// clamped to at least 1 sample so the ring math stays valid.
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            buf_l: vec![0.0; cap],
            buf_r: vec![0.0; cap],
            write_pos: 0,
            delay_samples_l: cap / 2,
            delay_samples_r: cap / 2,
            feedback: 0.0,
            ping_pong: false,
            filter_l: None,
            filter_r: None,
        }
    }

    /// Set the left-channel delay time in samples. Clamped to capacity.
    pub fn set_delay_l(&mut self, samples: usize) {
        self.delay_samples_l = samples.min(self.buf_l.len() - 1).max(1);
    }

    /// Set the right-channel delay time in samples. Clamped to capacity.
    pub fn set_delay_r(&mut self, samples: usize) {
        self.delay_samples_r = samples.min(self.buf_r.len() - 1).max(1);
    }

    /// Set both channels to the same delay (most common case).
    pub fn set_delay(&mut self, samples: usize) {
        self.set_delay_l(samples);
        self.set_delay_r(samples);
    }

    /// Compute the delay time in samples for a `num/den` tempo-sync
    /// division at the given BPM and sample rate.
    /// `num/den = 1/4` = quarter-note, `1/8` = eighth, etc.
    /// Dotted values: `3/16` = dotted eighth, `3/8` = dotted quarter.
    /// Triplets: `1/12` = quarter-note triplet (3 per half-note),
    /// `1/24` = eighth-note triplet (3 per quarter).
    pub fn tempo_sync_samples(bpm: f64, sample_rate: f64, num: u32, den: u32) -> usize {
        if bpm <= 0.0 || sample_rate <= 0.0 || den == 0 {
            return 0;
        }
        // A whole note is 4 beats; one beat = 60 / bpm seconds.
        let whole_note_secs = 4.0 * 60.0 / bpm;
        let secs = whole_note_secs * (num as f64) / (den as f64);
        (secs * sample_rate).round() as usize
    }

    /// Feedback amount in `[0.0, 1.0]`. 0.0 = no feedback, 1.0 = unity
    /// (infinite sustain — clamp carefully to avoid runaway).
    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = feedback.clamp(0.0, 0.99);
    }

    /// Enable ping-pong cross-feedback — feedback from the left tap
    /// feeds the right write position and vice versa, producing an
    /// alternating L/R echo pattern.
    pub fn set_ping_pong(&mut self, enabled: bool) {
        self.ping_pong = enabled;
    }

    /// Install a biquad filter on the feedback path for both channels.
    /// Pass `None` to remove the filter and return to unfiltered
    /// feedback. Typical usage: high-pass to clean low-end buildup,
    /// low-pass to tame bright repeats.
    pub fn set_feedback_filter(&mut self, filter: Option<Biquad>) {
        self.filter_l = filter;
        self.filter_r = filter;
    }

    /// Clear the ring buffers and filter state. Called when the
    /// transport jumps or when the user toggles bypass.
    pub fn reset(&mut self) {
        for s in self.buf_l.iter_mut() {
            *s = 0.0;
        }
        for s in self.buf_r.iter_mut() {
            *s = 0.0;
        }
        self.write_pos = 0;
        if let Some(f) = self.filter_l.as_mut() {
            f.reset();
        }
        if let Some(f) = self.filter_r.as_mut() {
            f.reset();
        }
    }

    /// Process a single stereo frame. Returns the wet output `(l, r)`
    /// — callers mix dry + wet themselves (use `distortion::parallel_mix`
    /// for a matched crossfade).
    pub fn process(&mut self, dry_l: f32, dry_r: f32) -> (f32, f32) {
        let cap = self.buf_l.len();
        let read_l = (self.write_pos + cap - self.delay_samples_l) % cap;
        let read_r = (self.write_pos + cap - self.delay_samples_r) % cap;
        let tap_l = self.buf_l[read_l];
        let tap_r = self.buf_r[read_r];

        // Apply optional feedback filter.
        let filtered_l = if let Some(f) = self.filter_l.as_mut() {
            f.process_mono(tap_l)
        } else {
            tap_l
        };
        let filtered_r = if let Some(f) = self.filter_r.as_mut() {
            f.process_mono(tap_r)
        } else {
            tap_r
        };

        // Cross-feed for ping-pong, straight feed otherwise.
        let (fb_into_l, fb_into_r) = if self.ping_pong {
            (filtered_r * self.feedback, filtered_l * self.feedback)
        } else {
            (filtered_l * self.feedback, filtered_r * self.feedback)
        };

        self.buf_l[self.write_pos] = dry_l + fb_into_l;
        self.buf_r[self.write_pos] = dry_r + fb_into_r;
        self.write_pos = (self.write_pos + 1) % cap;

        (tap_l, tap_r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_delays_by_configured_samples() {
        let mut d = StereoDelayLine::new(2048);
        d.set_delay(100);
        d.set_feedback(0.0);
        // Feed one impulse on tick 0, zeros after.
        let mut first_nonzero_tick = None;
        for t in 0..200 {
            let input = if t == 0 { 1.0 } else { 0.0 };
            let (l, _r) = d.process(input, 0.0);
            if l.abs() > 1e-6 && first_nonzero_tick.is_none() {
                first_nonzero_tick = Some(t);
            }
        }
        // The impulse should appear at the output at tick 100
        // (input sample 0 is read back after 100 samples of delay).
        assert_eq!(first_nonzero_tick, Some(100));
    }

    #[test]
    fn feedback_zero_produces_single_echo_only() {
        let mut d = StereoDelayLine::new(2048);
        d.set_delay(50);
        d.set_feedback(0.0);
        let mut nonzero_count = 0;
        for t in 0..500 {
            let input = if t == 0 { 1.0 } else { 0.0 };
            let (l, _r) = d.process(input, 0.0);
            if l.abs() > 1e-6 {
                nonzero_count += 1;
            }
        }
        assert_eq!(
            nonzero_count, 1,
            "zero feedback should produce exactly one echo"
        );
    }

    #[test]
    fn feedback_decays_over_repeats() {
        let mut d = StereoDelayLine::new(2048);
        d.set_delay(50);
        d.set_feedback(0.5);
        let mut amplitudes = Vec::new();
        for t in 0..1000 {
            let input = if t == 0 { 1.0 } else { 0.0 };
            let (l, _r) = d.process(input, 0.0);
            if l.abs() > 0.01 {
                amplitudes.push(l);
            }
        }
        // Each echo is half the previous: 1.0, 0.5, 0.25, 0.125, ...
        assert!(amplitudes.len() >= 3);
        for pair in amplitudes.windows(2) {
            assert!(pair[1] < pair[0], "echoes should decay: {pair:?}");
            let ratio = pair[1] / pair[0];
            assert!(
                (ratio - 0.5).abs() < 1e-3,
                "decay ratio {ratio} should be ~0.5 at feedback 0.5"
            );
        }
    }

    #[test]
    fn ping_pong_alternates_channels() {
        let mut d = StereoDelayLine::new(2048);
        d.set_delay(50);
        d.set_feedback(0.6);
        d.set_ping_pong(true);
        // Feed an impulse only on the left channel.
        let mut echoes_l = Vec::new();
        let mut echoes_r = Vec::new();
        for t in 0..500 {
            let input = if t == 0 { 1.0 } else { 0.0 };
            let (l, r) = d.process(input, 0.0);
            if l.abs() > 0.01 {
                echoes_l.push((t, l));
            }
            if r.abs() > 0.01 {
                echoes_r.push((t, r));
            }
        }
        // With ping-pong, the first echo lands on L (tap from original
        // input), then the feedback jumps to R via cross-feed.
        assert!(!echoes_l.is_empty());
        assert!(!echoes_r.is_empty(), "ping-pong should produce echoes on R");
    }

    #[test]
    fn tempo_sync_quarter_note_at_120bpm_matches_half_second() {
        // At 120 BPM a quarter note is 0.5 s = 24000 samples @ 48 kHz.
        let samples = StereoDelayLine::tempo_sync_samples(120.0, 48_000.0, 1, 4);
        assert_eq!(samples, 24_000);
    }

    #[test]
    fn tempo_sync_dotted_eighth_is_1_5x_eighth() {
        // Dotted eighth = 3/16; plain eighth = 1/8 = 2/16. Ratio 1.5.
        let dotted = StereoDelayLine::tempo_sync_samples(120.0, 48_000.0, 3, 16);
        let plain = StereoDelayLine::tempo_sync_samples(120.0, 48_000.0, 1, 8);
        let ratio = dotted as f64 / plain as f64;
        assert!((ratio - 1.5).abs() < 1e-3, "dotted/plain = {ratio}");
    }

    #[test]
    fn tempo_sync_handles_zero_and_negative_input() {
        assert_eq!(StereoDelayLine::tempo_sync_samples(0.0, 48_000.0, 1, 4), 0);
        assert_eq!(StereoDelayLine::tempo_sync_samples(120.0, 0.0, 1, 4), 0);
        assert_eq!(
            StereoDelayLine::tempo_sync_samples(120.0, 48_000.0, 1, 0),
            0
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut d = StereoDelayLine::new(512);
        d.set_delay(50);
        d.set_feedback(0.7);
        for _ in 0..100 {
            d.process(1.0, 1.0);
        }
        d.reset();
        // After reset, processing zero input should produce zero output.
        let (l, r) = d.process(0.0, 0.0);
        assert_eq!(l, 0.0);
        assert_eq!(r, 0.0);
    }
}
