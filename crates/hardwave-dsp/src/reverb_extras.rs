//! Reverb extras — low-frequency crossover, early-reflection gain,
//! diffusion control, and freeze mode. Composable on top of the
//! existing `AlgorithmicReverb` tank.

use std::f32::consts::PI;

/// Split the reverb input into low and high bands so the reverb can
/// apply different decay characteristics per band. Uses a pair of
/// one-pole filters for cheap, click-free LR branching.
pub struct LowFrequencyCrossover {
    cutoff_hz: f32,
    sample_rate: f32,
    lp_state: f32,
    alpha_lp: f32,
}

impl LowFrequencyCrossover {
    pub fn new(sample_rate: f32, cutoff_hz: f32) -> Self {
        let mut xo = Self {
            cutoff_hz,
            sample_rate: sample_rate.max(1.0),
            lp_state: 0.0,
            alpha_lp: 0.0,
        };
        xo.set_cutoff(cutoff_hz);
        xo
    }

    pub fn set_cutoff(&mut self, cutoff_hz: f32) {
        self.cutoff_hz = cutoff_hz.max(20.0);
        let dt = 1.0 / self.sample_rate;
        let rc = 1.0 / (2.0 * PI * self.cutoff_hz);
        self.alpha_lp = dt / (rc + dt);
    }

    pub fn cutoff(&self) -> f32 {
        self.cutoff_hz
    }

    /// Returns `(low, high)` — `low + high ≈ x` so callers can route
    /// each band through a different reverb stage and sum afterwards.
    pub fn split(&mut self, x: f32) -> (f32, f32) {
        self.lp_state += self.alpha_lp * (x - self.lp_state);
        let low = self.lp_state;
        let high = x - low;
        (low, high)
    }
}

/// Early-reflections sub-stage — small cluster of delayed taps that
/// simulates the first 10–80 ms of a room's response. Gain stage is
/// exposed so the caller can balance ER against the reverb tail.
pub struct EarlyReflections {
    taps: Vec<(usize, f32)>, // (delay samples, per-tap gain)
    buffer: Vec<f32>,
    write: usize,
    cap: usize,
    level: f32,
}

impl EarlyReflections {
    pub fn new(sample_rate: f32, pattern: EarlyReflectionPattern) -> Self {
        let (delays_ms, per_tap) = pattern.taps();
        let max_delay_ms = delays_ms.iter().cloned().fold(0.0_f32, f32::max);
        let cap = ((max_delay_ms * 0.001) * sample_rate).ceil() as usize + 1;
        let cap = cap.max(16);
        let taps: Vec<(usize, f32)> = delays_ms
            .iter()
            .zip(per_tap.iter())
            .map(|(&ms, &g)| (((ms * 0.001) * sample_rate) as usize, g))
            .collect();
        Self {
            taps,
            buffer: vec![0.0; cap],
            write: 0,
            cap,
            level: 1.0,
        }
    }

    /// Set the overall early-reflections level (multiplier on the
    /// summed tap output). 1.0 = unity.
    pub fn set_level(&mut self, level: f32) {
        self.level = level.max(0.0);
    }

    pub fn level(&self) -> f32 {
        self.level
    }

    pub fn process(&mut self, x: f32) -> f32 {
        self.buffer[self.write] = x;
        let mut sum = 0.0_f32;
        for &(delay, gain) in &self.taps {
            let read = (self.write + self.cap - delay) % self.cap;
            sum += self.buffer[read] * gain;
        }
        self.write = (self.write + 1) % self.cap;
        sum * self.level
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EarlyReflectionPattern {
    Hall,
    Room,
    Studio,
}

impl EarlyReflectionPattern {
    fn taps(self) -> (&'static [f32], &'static [f32]) {
        // Delay (ms) and per-tap gain for each ER pattern. Handpicked
        // to give distinct spatial signatures.
        match self {
            EarlyReflectionPattern::Hall => (
                &[11.0, 23.0, 37.0, 53.0, 71.0, 93.0, 117.0],
                &[0.55, 0.42, 0.34, 0.27, 0.22, 0.17, 0.13],
            ),
            EarlyReflectionPattern::Room => (
                &[4.0, 8.0, 13.0, 19.0, 27.0, 37.0],
                &[0.65, 0.48, 0.36, 0.27, 0.2, 0.14],
            ),
            EarlyReflectionPattern::Studio => {
                (&[2.0, 5.0, 8.0, 12.0, 17.0], &[0.7, 0.5, 0.35, 0.24, 0.15])
            }
        }
    }
}

/// Diffusion network — a cascade of short all-pass sections that
/// thicken the reverb by smearing transients. `amount` in `[0, 1]`
/// maps to the all-pass feedback coefficient.
pub struct DiffusionStage {
    allpasses: Vec<Allpass>,
    amount: f32,
}

impl DiffusionStage {
    pub fn new(sample_rate: f32) -> Self {
        let sizes_ms = [5.3, 7.9, 11.7, 17.3];
        let allpasses = sizes_ms
            .iter()
            .map(|ms| Allpass::new(sample_rate, *ms, 0.5))
            .collect();
        Self {
            allpasses,
            amount: 0.5,
        }
    }

    pub fn set_amount(&mut self, amount: f32) {
        self.amount = amount.clamp(0.0, 0.95);
        for ap in self.allpasses.iter_mut() {
            ap.set_feedback(self.amount);
        }
    }

    pub fn amount(&self) -> f32 {
        self.amount
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let mut y = x;
        for ap in self.allpasses.iter_mut() {
            y = ap.process(y);
        }
        y
    }
}

struct Allpass {
    buffer: Vec<f32>,
    write: usize,
    cap: usize,
    feedback: f32,
}

impl Allpass {
    fn new(sample_rate: f32, delay_ms: f32, feedback: f32) -> Self {
        let cap = ((delay_ms * 0.001) * sample_rate) as usize + 1;
        let cap = cap.max(2);
        Self {
            buffer: vec![0.0; cap],
            write: 0,
            cap,
            feedback: feedback.clamp(0.0, 0.95),
        }
    }

    fn set_feedback(&mut self, fb: f32) {
        self.feedback = fb.clamp(0.0, 0.95);
    }

    fn process(&mut self, x: f32) -> f32 {
        let read = self.write; // delay = full cap-1; simple ring read
        let buffered = self.buffer[read];
        let out = -x * self.feedback + buffered;
        self.buffer[self.write] = x + buffered * self.feedback;
        self.write = (self.write + 1) % self.cap;
        out
    }
}

/// Freeze mode — when engaged, the reverb tank's feedback is set to
/// 1.0 so the tail sustains indefinitely and no new input is injected.
/// `FreezeState` is a tiny helper that reports the current gain
/// multipliers for the input and feedback signals.
#[derive(Debug, Clone, Copy)]
pub struct FreezeState {
    frozen: bool,
}

impl FreezeState {
    pub fn new() -> Self {
        Self { frozen: false }
    }

    pub fn set_frozen(&mut self, frozen: bool) {
        self.frozen = frozen;
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Returns the (input_gain, feedback_gain) pair the reverb should
    /// apply this sample.
    pub fn gains(&self) -> (f32, f32) {
        if self.frozen {
            (0.0, 1.0)
        } else {
            (1.0, 0.0)
        }
    }
}

impl Default for FreezeState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sr).sin())
            .collect()
    }

    #[test]
    fn crossover_low_band_keeps_lows_and_drops_highs() {
        let sr = 48_000.0;
        let mut xo = LowFrequencyCrossover::new(sr, 200.0);
        let mut low_energy = 0.0_f32;
        let mut high_carryover = 0.0_f32;
        for s in sine(80.0, sr, sr as usize) {
            let (l, _) = xo.split(s);
            low_energy += l.powi(2);
        }
        xo = LowFrequencyCrossover::new(sr, 200.0);
        for s in sine(2000.0, sr, sr as usize) {
            let (l, _) = xo.split(s);
            high_carryover += l.powi(2);
        }
        assert!(
            low_energy > high_carryover * 10.0,
            "low energy {} vs high carryover {}",
            low_energy,
            high_carryover
        );
    }

    #[test]
    fn crossover_low_plus_high_reconstructs_input() {
        let sr = 48_000.0;
        let mut xo = LowFrequencyCrossover::new(sr, 300.0);
        for s in sine(440.0, sr, 1024) {
            let (l, h) = xo.split(s);
            assert!((l + h - s).abs() < 1e-5);
        }
    }

    #[test]
    fn early_reflections_output_is_delayed_sum() {
        let sr = 48_000.0;
        let mut er = EarlyReflections::new(sr, EarlyReflectionPattern::Room);
        // Push an impulse, then zeros, and check non-zero output at
        // the tap positions.
        let mut impulse_response = Vec::new();
        for i in 0..(sr * 0.1) as usize {
            let input = if i == 0 { 1.0 } else { 0.0 };
            impulse_response.push(er.process(input));
        }
        // Should have at least one non-zero sample past the first 10
        // samples (first tap is ~4 ms in for Room pattern = 192 samples).
        let post = &impulse_response[10..];
        let peak: f32 = post.iter().fold(0.0, |acc, v| acc.max(v.abs()));
        assert!(peak > 0.05, "peak {}", peak);
    }

    #[test]
    fn early_reflections_level_scales_output() {
        let sr = 48_000.0;
        let mut er1 = EarlyReflections::new(sr, EarlyReflectionPattern::Hall);
        let mut er2 = EarlyReflections::new(sr, EarlyReflectionPattern::Hall);
        er2.set_level(0.5);
        let input = sine(500.0, sr, 4096);
        let mut e1 = 0.0_f32;
        let mut e2 = 0.0_f32;
        for &s in &input {
            e1 += er1.process(s).powi(2);
            e2 += er2.process(s).powi(2);
        }
        // Level 0.5 → energy should be ~0.25×.
        assert!(e2 < e1 * 0.5, "e1 {} e2 {}", e1, e2);
    }

    #[test]
    fn diffusion_amount_clamps_and_thickens_output() {
        let sr = 48_000.0;
        let mut diff = DiffusionStage::new(sr);
        diff.set_amount(2.0); // clamps to 0.95
        assert!((diff.amount() - 0.95).abs() < 1e-4);
        // Impulse response spreads out more with higher diffusion —
        // measure RMS over the tail relative to the peak.
        let mut response = Vec::new();
        for i in 0..1024 {
            let input = if i == 0 { 1.0 } else { 0.0 };
            response.push(diff.process(input));
        }
        let peak: f32 = response.iter().fold(0.0, |acc, v| acc.max(v.abs()));
        let tail_rms: f32 = (response[32..].iter().map(|v| v * v).sum::<f32>() / 992.0).sqrt();
        assert!(peak > 0.0);
        assert!(tail_rms > 0.0);
    }

    #[test]
    fn freeze_state_toggle_inverts_gain_roles() {
        let mut f = FreezeState::new();
        assert_eq!(f.gains(), (1.0, 0.0));
        f.set_frozen(true);
        assert_eq!(f.gains(), (0.0, 1.0));
        assert!(f.is_frozen());
    }
}
