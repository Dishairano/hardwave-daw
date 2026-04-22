//! Wavetable synth primitives — `Wavetable` holds a set of
//! single-cycle waveforms in a contiguous buffer and reads through
//! them via a `position` parameter (0..1 scans across the table).
//! `WavetableOscillator` wraps a wavetable with phase tracking,
//! pitch control, and position modulation.

use std::f32::consts::PI;

/// Single-cycle table length. Power of 2 for fast modulo via `&`.
pub const WAVE_LENGTH: usize = 2048;

/// A wavetable — N frames of WAVE_LENGTH samples each. `position` in
/// [0, 1] selects (and linearly interpolates between) frames.
pub struct Wavetable {
    frames: Vec<Vec<f32>>,
}

impl Wavetable {
    /// Build a wavetable from a set of single-cycle waveforms.
    /// Each slice must be WAVE_LENGTH samples long; shorter inputs
    /// are zero-padded, longer are truncated.
    pub fn from_frames<I>(frames: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<[f32]>,
    {
        let mut out = Vec::new();
        for frame in frames {
            let frame = frame.as_ref();
            let mut v = vec![0.0; WAVE_LENGTH];
            let n = frame.len().min(WAVE_LENGTH);
            v[..n].copy_from_slice(&frame[..n]);
            out.push(v);
        }
        if out.is_empty() {
            out.push(vec![0.0; WAVE_LENGTH]);
        }
        Self { frames: out }
    }

    /// Built-in "basic" wavetable: sine → triangle → saw → square.
    pub fn basic() -> Self {
        let mut frames = Vec::with_capacity(4);
        let mut sine = vec![0.0; WAVE_LENGTH];
        let mut tri = vec![0.0; WAVE_LENGTH];
        let mut saw = vec![0.0; WAVE_LENGTH];
        let mut sq = vec![0.0; WAVE_LENGTH];
        for i in 0..WAVE_LENGTH {
            let p = i as f32 / WAVE_LENGTH as f32;
            sine[i] = (2.0 * PI * p).sin();
            tri[i] = if p < 0.5 {
                4.0 * p - 1.0
            } else {
                3.0 - 4.0 * p
            };
            saw[i] = 2.0 * p - 1.0;
            sq[i] = if p < 0.5 { 1.0 } else { -1.0 };
        }
        frames.push(sine);
        frames.push(tri);
        frames.push(saw);
        frames.push(sq);
        Self { frames }
    }

    /// Built-in "analog" wavetable: soft saw → ramp → pulse-width
    /// sweeping square → mellow triangle with subtle harmonic
    /// drift. Emulates the warmer edge of classic VA oscillators.
    pub fn analog() -> Self {
        let mut frames: Vec<Vec<f32>> = Vec::with_capacity(4);
        for stage in 0..4 {
            let mut v = vec![0.0_f32; WAVE_LENGTH];
            for (i, sample) in v.iter_mut().enumerate() {
                let p = i as f32 / WAVE_LENGTH as f32;
                let t = 2.0 * PI * p;
                *sample = match stage {
                    // Soft saw: band-limited additive out to 16 harmonics.
                    0 => band_limited_saw(p, 16),
                    // Asymmetric ramp with fundamental-stacked soft rolloff.
                    1 => band_limited_saw(p, 8) * 0.7 + (t).sin() * 0.3,
                    // PWM square at 40% duty.
                    2 => {
                        if p < 0.4 {
                            0.9
                        } else {
                            -0.9
                        }
                    }
                    // Mellow triangle with a touch of 3rd harmonic.
                    _ => {
                        let tri = if p < 0.5 {
                            4.0 * p - 1.0
                        } else {
                            3.0 - 4.0 * p
                        };
                        tri * 0.85 + (3.0 * t).sin() * 0.1
                    }
                };
            }
            frames.push(normalize_frame(v));
        }
        Self { frames }
    }

    /// Built-in "digital" wavetable: harsh, high-harmonic-content
    /// frames reminiscent of FM / PD / DX-style oscillators —
    /// double-saw stack, bit-reduced staircase, sawtooth-sync, and
    /// square-with-extra-odd-harmonics.
    pub fn digital() -> Self {
        let mut frames: Vec<Vec<f32>> = Vec::with_capacity(4);
        for stage in 0..4 {
            let mut v = vec![0.0_f32; WAVE_LENGTH];
            for (i, sample) in v.iter_mut().enumerate() {
                let p = i as f32 / WAVE_LENGTH as f32;
                let t = 2.0 * PI * p;
                *sample = match stage {
                    // Double saw (super-saw-ish stack of two slightly
                    // detuned saws).
                    0 => band_limited_saw(p, 32) * 0.6 + band_limited_saw(p * 1.01, 16) * 0.4,
                    // 4-bit staircase.
                    1 => {
                        let steps = 16.0;
                        ((p * steps).floor() / steps) * 2.0 - 1.0 + 1.0 / steps
                    }
                    // Hard-sync saw at ratio 2 (second saw resets twice
                    // per cycle — bright, buzzy).
                    2 => band_limited_saw(((p * 2.0) % 1.0 + 1.0) % 1.0, 16),
                    // Square + odd-harmonic flavor (7th harmonic kick).
                    _ => {
                        let sq = if p < 0.5 { 1.0 } else { -1.0 };
                        sq * 0.8 + (7.0 * t).sin() * 0.2
                    }
                };
            }
            frames.push(normalize_frame(v));
        }
        Self { frames }
    }

    /// Built-in "vocal" wavetable: four frames approximating the
    /// vowels /a/, /e/, /i/, /o/ via sums of formant-tuned sine
    /// partials. Morphing across position sweeps the vowel.
    pub fn vocal() -> Self {
        // F1 / F2 formant pairs (Hz) for a male voice, referenced to
        // a fundamental of 110 Hz. We bake the partials nearest each
        // formant into the single-cycle table.
        let vowels: [(f32, f32); 4] = [
            (730.0, 1090.0), // /a/
            (530.0, 1840.0), // /e/
            (270.0, 2290.0), // /i/
            (570.0, 840.0),  // /o/
        ];
        let f0 = 110.0_f32;
        let mut frames: Vec<Vec<f32>> = Vec::with_capacity(vowels.len());
        for (f1, f2) in vowels.iter().copied() {
            let mut v = vec![0.0_f32; WAVE_LENGTH];
            for (i, sample) in v.iter_mut().enumerate() {
                let p = i as f32 / WAVE_LENGTH as f32;
                let t = 2.0 * PI * p;
                let mut s = 0.0_f32;
                // Sum the first 24 harmonics with gain peaked at the
                // two formant frequencies.
                for k in 1..=24 {
                    let freq = (k as f32) * f0;
                    let gain = formant_gain(freq, f1, 80.0) + formant_gain(freq, f2, 120.0);
                    if gain > 0.001 {
                        s += (k as f32 * t).sin() * gain;
                    }
                }
                *sample = s;
            }
            frames.push(normalize_frame(v));
        }
        Self { frames }
    }

    /// Built-in "noise" wavetable: one frame of deterministic pseudo-
    /// random noise. Useful as a fourth oscillator source.
    pub fn noise() -> Self {
        let mut v = vec![0.0; WAVE_LENGTH];
        let mut state: u32 = 0xDEADBEEF;
        for sample in v.iter_mut() {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *sample = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
        }
        Self { frames: vec![v] }
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Sample the table at a given `phase` (0..1) and `position` (0..1).
    /// Linearly interpolates both within a frame and between frames.
    pub fn sample(&self, phase: f32, position: f32) -> f32 {
        let pos = position.clamp(0.0, 1.0);
        if self.frames.len() == 1 {
            return Self::sample_frame(&self.frames[0], phase);
        }
        let f_scaled = pos * (self.frames.len() - 1) as f32;
        let idx_low = f_scaled.floor() as usize;
        let idx_high = (idx_low + 1).min(self.frames.len() - 1);
        let frac = f_scaled - idx_low as f32;
        let a = Self::sample_frame(&self.frames[idx_low], phase);
        let b = Self::sample_frame(&self.frames[idx_high], phase);
        a + (b - a) * frac
    }

    fn sample_frame(frame: &[f32], phase: f32) -> f32 {
        let p = phase.rem_euclid(1.0) * WAVE_LENGTH as f32;
        let idx = p as usize;
        let frac = p - idx as f32;
        let a = frame[idx & (WAVE_LENGTH - 1)];
        let b = frame[(idx + 1) & (WAVE_LENGTH - 1)];
        a + (b - a) * frac
    }
}

fn band_limited_saw(phase: f32, harmonics: usize) -> f32 {
    let t = 2.0 * PI * phase;
    let mut s = 0.0_f32;
    for k in 1..=harmonics {
        s += (k as f32 * t).sin() / k as f32;
    }
    // Scale down so harmonic sums stay in range.
    -2.0 / PI * s
}

fn normalize_frame(mut frame: Vec<f32>) -> Vec<f32> {
    let peak = frame.iter().fold(0.0_f32, |acc, v| acc.max(v.abs()));
    if peak > 1e-6 {
        let inv = 0.98 / peak;
        for v in frame.iter_mut() {
            *v *= inv;
        }
    }
    frame
}

fn formant_gain(freq: f32, center: f32, bandwidth: f32) -> f32 {
    // Gaussian-shaped formant peak — unit gain at `center`, falls off
    // with distance scaled by `bandwidth`.
    let x = (freq - center) / bandwidth;
    (-0.5 * x * x).exp()
}

/// Wavetable oscillator — holds a reference to a `Wavetable` via
/// index into a caller-owned table list, plus its own phase / pitch /
/// position state.
pub struct WavetableOscillator {
    phase: f32,
    frequency_hz: f32,
    sample_rate: f32,
    level: f32,
    position: f32,
}

impl WavetableOscillator {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            phase: 0.0,
            frequency_hz: 440.0,
            sample_rate: sample_rate.max(1.0),
            level: 1.0,
            position: 0.0,
        }
    }

    pub fn set_frequency(&mut self, hz: f32) {
        self.frequency_hz = hz.max(0.0);
    }

    pub fn set_level(&mut self, level: f32) {
        self.level = level.clamp(0.0, 4.0);
    }

    /// Position in the wavetable (0..1). Use modulated positions for
    /// sweeping through waveforms over time (LFO or envelope drive).
    pub fn set_position(&mut self, position: f32) {
        self.position = position.clamp(0.0, 1.0);
    }

    pub fn position(&self) -> f32 {
        self.position
    }

    pub fn reset_phase(&mut self) {
        self.phase = 0.0;
    }

    /// Advance one sample and return the oscillator's output sampled
    /// from the given wavetable.
    pub fn tick(&mut self, table: &Wavetable) -> f32 {
        let out = table.sample(self.phase, self.position) * self.level;
        self.phase = (self.phase + self.frequency_hz / self.sample_rate).rem_euclid(1.0);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_wavetable_has_four_frames() {
        let wt = Wavetable::basic();
        assert_eq!(wt.frame_count(), 4);
    }

    #[test]
    fn sine_frame_matches_analytic_sine() {
        let wt = Wavetable::basic();
        // Position 0 selects the sine frame. Sample at phase 0.25 → should ≈ 1.0.
        let s0 = wt.sample(0.0, 0.0);
        let s_q = wt.sample(0.25, 0.0);
        let s_h = wt.sample(0.5, 0.0);
        let s_3q = wt.sample(0.75, 0.0);
        assert!(s0.abs() < 0.01, "sine(0) = {s0}");
        assert!((s_q - 1.0).abs() < 0.01, "sine(0.25) = {s_q}");
        assert!(s_h.abs() < 0.01, "sine(0.5) = {s_h}");
        assert!((s_3q + 1.0).abs() < 0.01, "sine(0.75) = {s_3q}");
    }

    #[test]
    fn position_interpolates_between_adjacent_frames() {
        // Halfway between sine (frame 0) and triangle (frame 1) at phase 0
        // should be halfway between sine(0)=0 and triangle(0)=-1, i.e. -0.5.
        let wt = Wavetable::basic();
        let mid_pos = 1.0 / 3.0 * 0.5; // halfway between frame 0 and 1 out of 4
        let s = wt.sample(0.0, mid_pos);
        // sine(0) = 0, triangle(0) = -1; halfway = -0.5.
        assert!(
            (s + 0.5).abs() < 0.1,
            "midpoint sample = {s}, expected ≈ -0.5"
        );
    }

    #[test]
    fn noise_wavetable_has_one_frame() {
        let wt = Wavetable::noise();
        assert_eq!(wt.frame_count(), 1);
        // Noise values should span roughly [-1, 1] across the frame.
        let mut max = f32::MIN;
        let mut min = f32::MAX;
        for i in 0..100 {
            let s = wt.sample(i as f32 / 100.0, 0.0);
            if s > max {
                max = s;
            }
            if s < min {
                min = s;
            }
        }
        assert!(max > 0.5 && min < -0.5, "noise range = [{min}, {max}]");
    }

    #[test]
    fn wavetable_oscillator_advances_phase() {
        let wt = Wavetable::basic();
        let mut osc = WavetableOscillator::new(48_000.0);
        osc.set_frequency(100.0);
        osc.set_position(0.0); // sine
        let samples: Vec<f32> = (0..500).map(|_| osc.tick(&wt)).collect();
        let max = samples.iter().cloned().fold(f32::MIN, f32::max);
        let min = samples.iter().cloned().fold(f32::MAX, f32::min);
        assert!(max > 0.99 && min < -0.99, "osc output = [{min}, {max}]");
    }

    #[test]
    fn position_sweep_changes_timbre() {
        let wt = Wavetable::basic();
        let mut osc = WavetableOscillator::new(48_000.0);
        osc.set_frequency(1000.0);
        osc.set_position(0.0); // pure sine
        let sine_samples: Vec<f32> = (0..500).map(|_| osc.tick(&wt)).collect();
        osc.reset_phase();
        osc.set_position(1.0); // pure square
        let sq_samples: Vec<f32> = (0..500).map(|_| osc.tick(&wt)).collect();
        // Sine RMS should be ~0.707; square RMS ~1.0.
        fn rms(x: &[f32]) -> f32 {
            (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
        }
        let sine_rms = rms(&sine_samples);
        let sq_rms = rms(&sq_samples);
        assert!(
            sq_rms > sine_rms,
            "square RMS {sq_rms} should exceed sine RMS {sine_rms}"
        );
    }

    #[test]
    fn custom_frames_build_correctly() {
        let frame1 = vec![0.5; WAVE_LENGTH];
        let frame2 = vec![-0.5; WAVE_LENGTH];
        let wt = Wavetable::from_frames([frame1.as_slice(), frame2.as_slice()]);
        assert_eq!(wt.frame_count(), 2);
        assert!((wt.sample(0.0, 0.0) - 0.5).abs() < 0.01);
        assert!((wt.sample(0.0, 1.0) - (-0.5)).abs() < 0.01);
    }

    #[test]
    fn analog_wavetable_has_four_frames_and_is_bounded() {
        let wt = Wavetable::analog();
        assert_eq!(wt.frame_count(), 4);
        for frame_pos in [0.0, 0.33, 0.66, 1.0] {
            for i in 0..256 {
                let s = wt.sample(i as f32 / 256.0, frame_pos);
                assert!(s.abs() <= 1.01, "|sample| {} at pos {}", s, frame_pos);
            }
        }
    }

    #[test]
    fn digital_wavetable_has_four_frames_and_is_not_silent() {
        let wt = Wavetable::digital();
        assert_eq!(wt.frame_count(), 4);
        for frame_pos in [0.0, 0.33, 0.66, 1.0] {
            let energy: f32 = (0..256)
                .map(|i| wt.sample(i as f32 / 256.0, frame_pos).powi(2))
                .sum();
            assert!(energy > 1.0, "frame {} silent", frame_pos);
        }
    }

    #[test]
    fn vocal_wavetable_has_four_frames_and_is_distinct_per_vowel() {
        let wt = Wavetable::vocal();
        assert_eq!(wt.frame_count(), 4);
        let sample_at =
            |pos: f32| -> Vec<f32> { (0..256).map(|i| wt.sample(i as f32 / 256.0, pos)).collect() };
        let a = sample_at(0.0);
        let e = sample_at(1.0 / 3.0);
        let diff: f32 = a.iter().zip(e.iter()).map(|(x, y)| (x - y).abs()).sum();
        assert!(diff > 10.0, "vowels not distinct enough, diff {}", diff);
    }
}
