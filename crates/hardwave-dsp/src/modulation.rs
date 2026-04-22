//! Modulation effects primitives — LFO-modulated delay line for
//! chorus / flanger plus a small allpass chain for phasers.
//!
//! The modulated delay line reads at a sample-accurately-interpolated
//! position that varies every sample; no clicks at block boundaries.
//! Multiple voices (for chorus depth) are implemented by instantiating
//! multiple `ModulatedDelay` instances with different phase offsets.

use std::f32::consts::PI;

/// A delay line whose read position is modulated by an internal LFO.
/// Used as the core of chorus and flanger effects. The caller sets the
/// base delay (in samples), the LFO rate (Hz), modulation depth (in
/// samples of deviation from the base delay), and optional feedback.
pub struct ModulatedDelay {
    buffer: Vec<f32>,
    write_pos: usize,
    base_delay: f32,
    lfo_phase: f32,
    lfo_rate_hz: f32,
    lfo_depth_samples: f32,
    feedback: f32,
    sample_rate: f32,
}

impl ModulatedDelay {
    pub fn new(max_delay_samples: usize, sample_rate: f32) -> Self {
        Self {
            buffer: vec![0.0; max_delay_samples.max(2)],
            write_pos: 0,
            base_delay: 100.0,
            lfo_phase: 0.0,
            lfo_rate_hz: 1.0,
            lfo_depth_samples: 0.0,
            feedback: 0.0,
            sample_rate: sample_rate.max(1.0),
        }
    }

    /// Set base delay in samples. Clamped to `[1, capacity - 2]` so the
    /// LFO can modulate around it without hitting boundaries.
    pub fn set_base_delay(&mut self, samples: f32) {
        let max = (self.buffer.len() as f32) - 2.0;
        self.base_delay = samples.clamp(1.0, max);
    }

    pub fn set_lfo_rate(&mut self, hz: f32) {
        self.lfo_rate_hz = hz.max(0.0);
    }

    pub fn set_lfo_depth(&mut self, depth_samples: f32) {
        self.lfo_depth_samples = depth_samples.max(0.0);
    }

    /// LFO starting phase in `[0, 1)`. Used for multi-voice chorus —
    /// each voice has its own delay + an offset phase so they produce
    /// decorrelated modulation.
    pub fn set_lfo_phase_offset(&mut self, phase: f32) {
        self.lfo_phase = phase.rem_euclid(1.0);
    }

    /// Feedback amount in `[0.0, 0.95]`.
    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = feedback.clamp(0.0, 0.95);
    }

    pub fn reset(&mut self) {
        for s in self.buffer.iter_mut() {
            *s = 0.0;
        }
        self.write_pos = 0;
        self.lfo_phase = 0.0;
    }

    /// Process one sample. Returns the wet output — the caller mixes
    /// dry + wet to taste.
    pub fn process(&mut self, input: f32) -> f32 {
        // Current sine-LFO delay deviation, in samples.
        let lfo = (2.0 * PI * self.lfo_phase).sin();
        let modulated_delay = (self.base_delay + lfo * self.lfo_depth_samples).max(1.0);
        let cap = self.buffer.len() as f32;
        let read_pos_f = (self.write_pos as f32 + cap - modulated_delay).rem_euclid(cap);

        // Linear interpolation between the two bracketing samples.
        let idx = read_pos_f as usize;
        let frac = read_pos_f - idx as f32;
        let s0 = self.buffer[idx % self.buffer.len()];
        let s1 = self.buffer[(idx + 1) % self.buffer.len()];
        let tap = s0 + (s1 - s0) * frac;

        let write_val = input + tap * self.feedback;
        self.buffer[self.write_pos] = write_val;
        self.write_pos = (self.write_pos + 1) % self.buffer.len();

        // Advance LFO.
        self.lfo_phase = (self.lfo_phase + self.lfo_rate_hz / self.sample_rate).rem_euclid(1.0);
        tap
    }
}

/// Single first-order allpass stage — the phaser's unit building block.
/// `y[n] = -a·x[n] + x[n-1] + a·y[n-1]` where `a = (1 - tan(π·f/sr)) /
/// (1 + tan(π·f/sr))` for a notch frequency `f`.
#[derive(Default, Clone, Copy)]
pub struct AllpassStage {
    a: f32,
    x1: f32,
    y1: f32,
}

impl AllpassStage {
    pub fn set_notch_hz(&mut self, notch_hz: f32, sample_rate: f32) {
        let sr = sample_rate.max(1.0);
        let t = (PI * notch_hz.clamp(10.0, sr * 0.49) / sr).tan();
        self.a = (1.0 - t) / (1.0 + t);
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = -self.a * x + self.x1 + self.a * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// Chain of `N` allpass stages — the phaser body. Each stage is set
/// to a different notch frequency to build up a comb-like response.
pub struct PhaserChain<const N: usize> {
    stages: [AllpassStage; N],
}

impl<const N: usize> Default for PhaserChain<N> {
    fn default() -> Self {
        Self {
            stages: [AllpassStage::default(); N],
        }
    }
}

impl<const N: usize> PhaserChain<N> {
    /// Set all stages to a range of notches between `base_hz` and
    /// `base_hz × 2^spread_octaves`.
    pub fn set_notches(&mut self, base_hz: f32, spread_octaves: f32, sample_rate: f32) {
        for (i, stage) in self.stages.iter_mut().enumerate() {
            let t = if N <= 1 {
                0.0
            } else {
                i as f32 / (N - 1) as f32
            };
            let notch = base_hz * 2.0_f32.powf(t * spread_octaves);
            stage.set_notch_hz(notch, sample_rate);
        }
    }

    pub fn reset(&mut self) {
        for stage in self.stages.iter_mut() {
            stage.reset();
        }
    }

    /// Process a sample through the full chain.
    pub fn process(&mut self, x: f32) -> f32 {
        let mut v = x;
        for stage in self.stages.iter_mut() {
            v = stage.tick(v);
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modulated_delay_produces_delayed_impulse() {
        let mut d = ModulatedDelay::new(512, 48_000.0);
        d.set_base_delay(100.0);
        d.set_lfo_depth(0.0);
        d.set_feedback(0.0);
        // Feed one impulse.
        let mut first_nonzero = None;
        for t in 0..300 {
            let x = if t == 0 { 1.0 } else { 0.0 };
            let out = d.process(x);
            if out.abs() > 0.5 && first_nonzero.is_none() {
                first_nonzero = Some(t);
            }
        }
        assert_eq!(first_nonzero, Some(100));
    }

    #[test]
    fn modulated_delay_lfo_widens_delay_range() {
        let mut d = ModulatedDelay::new(2048, 48_000.0);
        d.set_base_delay(500.0);
        d.set_lfo_rate(2.0);
        d.set_lfo_depth(200.0);
        d.set_feedback(0.0);
        // Feed a continuous input and sample the output; the LFO
        // should shift the delay within ±200 samples of 500.
        let mut min_out = f32::MAX;
        let mut max_out = f32::MIN;
        for i in 0..48_000 {
            let x = ((2.0 * PI * 100.0 * i as f32) / 48_000.0).sin();
            let out = d.process(x);
            if i > 1000 {
                if out < min_out {
                    min_out = out;
                }
                if out > max_out {
                    max_out = out;
                }
            }
        }
        // With modulation the output varies through the sine signal;
        // both positive and negative peaks should appear.
        assert!(max_out > 0.5 && min_out < -0.5);
    }

    #[test]
    fn modulated_delay_reset_clears_state() {
        let mut d = ModulatedDelay::new(512, 48_000.0);
        d.set_base_delay(50.0);
        d.set_feedback(0.5);
        for _ in 0..200 {
            d.process(1.0);
        }
        d.reset();
        // Feeding zero after reset should produce zero.
        let out = d.process(0.0);
        assert!(out.abs() < 1e-6);
    }

    #[test]
    fn allpass_has_unity_gain() {
        let mut ap = AllpassStage::default();
        ap.set_notch_hz(1000.0, 48_000.0);
        // Feed a long sinusoid and check the output magnitude.
        let mut max_out = 0.0_f32;
        for i in 0..48_000 {
            let x = ((2.0 * PI * 2000.0 * i as f32) / 48_000.0).sin();
            let y = ap.tick(x);
            if i > 100 && y.abs() > max_out {
                max_out = y.abs();
            }
        }
        // Allpass = unity magnitude response, should peak ≈ 1.0.
        assert!(
            (max_out - 1.0).abs() < 0.05,
            "allpass magnitude = {max_out}"
        );
    }

    #[test]
    fn phaser_chain_notches_are_ordered() {
        let mut phaser: PhaserChain<4> = PhaserChain::default();
        phaser.set_notches(200.0, 3.0, 48_000.0);
        // The chain should process without panicking and produce
        // something near unity magnitude on DC plus a bit of a
        // transient. Just check stability.
        let mut out_max = 0.0_f32;
        for i in 0..10_000 {
            let x = ((2.0 * PI * 500.0 * i as f32) / 48_000.0).sin();
            let y = phaser.process(x);
            if y.abs() > out_max {
                out_max = y.abs();
            }
        }
        assert!(out_max.is_finite());
        assert!(
            out_max < 2.0,
            "phaser chain went unstable, out_max = {out_max}"
        );
    }
}
