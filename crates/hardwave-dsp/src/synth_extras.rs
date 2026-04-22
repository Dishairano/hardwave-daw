//! Subtractive-synth extras — filter drive saturation, key-tracked
//! cutoff offset, master volume + pan, and a built-in FX chain
//! (chorus → delay → reverb) with simple on/off toggles. Composes
//! on top of the existing `synth`, `biquad`, `modulation`, and
//! `reverb` primitives.

use std::f32::consts::PI;

/// Soft-clip drive stage applied before the filter. `amount` in
/// `[0, 1]` maps from "transparent" to "hot" — 0 is unity gain, 1
/// adds ~12 dB of gain + aggressive tanh saturation.
pub struct FilterDrive {
    amount: f32,
}

impl FilterDrive {
    pub fn new() -> Self {
        Self { amount: 0.0 }
    }

    pub fn set_amount(&mut self, amount: f32) {
        self.amount = amount.clamp(0.0, 1.0);
    }

    pub fn amount(&self) -> f32 {
        self.amount
    }

    pub fn process(&self, x: f32) -> f32 {
        if self.amount <= 1e-4 {
            return x;
        }
        let pre_gain_db = 12.0 * self.amount;
        let pre = x * 10_f32.powf(pre_gain_db / 20.0);
        // Compensate post-gain so loud patches don't bloom.
        let post = (pre).tanh();
        post * (1.0 - self.amount * 0.2)
    }
}

impl Default for FilterDrive {
    fn default() -> Self {
        Self::new()
    }
}

/// Key-tracking helper — cutoff frequency tracks the MIDI note by a
/// fractional amount. `0.0` = flat (cutoff never moves); `1.0` =
/// full tracking (cutoff doubles per octave). `base_cutoff` is the
/// cutoff at the reference note (A4 = 69).
pub fn key_tracked_cutoff(base_cutoff_hz: f32, midi_note: f32, amount: f32) -> f32 {
    let amount = amount.clamp(0.0, 1.0);
    let octaves_above_ref = (midi_note - 69.0) / 12.0 * amount;
    (base_cutoff_hz * 2_f32.powf(octaves_above_ref)).clamp(20.0, 20_000.0)
}

/// Master output strip — volume in dB + pan in `[-1, 1]`. Produces
/// a stereo `(left, right)` pair using equal-power pan law.
pub struct MasterStrip {
    volume_linear: f32,
    pan: f32,
}

impl MasterStrip {
    pub fn new() -> Self {
        Self {
            volume_linear: 1.0,
            pan: 0.0,
        }
    }

    pub fn set_volume_db(&mut self, db: f32) {
        let clamped = db.clamp(-60.0, 12.0);
        self.volume_linear = 10_f32.powf(clamped / 20.0);
    }

    pub fn set_pan(&mut self, pan: f32) {
        self.pan = pan.clamp(-1.0, 1.0);
    }

    pub fn volume_linear(&self) -> f32 {
        self.volume_linear
    }

    pub fn pan(&self) -> f32 {
        self.pan
    }

    /// Mono input → stereo output using equal-power pan law.
    pub fn process_mono(&self, x: f32) -> (f32, f32) {
        let angle = (self.pan + 1.0) * 0.25 * PI; // maps -1..1 → 0..π/2
        let (s, c) = angle.sin_cos();
        let scaled = x * self.volume_linear;
        (scaled * c, scaled * s)
    }
}

impl Default for MasterStrip {
    fn default() -> Self {
        Self::new()
    }
}

/// Built-in FX chain for the subtractive synth — chorus, delay,
/// reverb. Each effect has a simple `enabled` flag and a `mix`
/// control so the caller can dial in subtle character without
/// leaving the synth UI.
pub struct SynthBuiltInFx {
    chorus: SimpleChorus,
    delay: SimpleDelay,
    reverb: SimpleReverb,
}

impl SynthBuiltInFx {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            chorus: SimpleChorus::new(sample_rate),
            delay: SimpleDelay::new(sample_rate),
            reverb: SimpleReverb::new(sample_rate),
        }
    }

    pub fn chorus_mut(&mut self) -> &mut SimpleChorus {
        &mut self.chorus
    }

    pub fn delay_mut(&mut self) -> &mut SimpleDelay {
        &mut self.delay
    }

    pub fn reverb_mut(&mut self) -> &mut SimpleReverb {
        &mut self.reverb
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let c = self.chorus.process(x);
        let d = self.delay.process(c);
        self.reverb.process(d)
    }
}

/// A simple modulated-delay chorus — single voice, 15 ms base delay
/// with a 0.6 Hz sine LFO modulating ±3 ms. Good enough as a built-
/// in preset; users who want a real chorus chain Ensemble / Chorus
/// plugins separately.
pub struct SimpleChorus {
    buffer: Vec<f32>,
    write: usize,
    cap: usize,
    sample_rate: f32,
    phase: f32,
    rate_hz: f32,
    depth_ms: f32,
    base_ms: f32,
    mix: f32,
    enabled: bool,
}

impl SimpleChorus {
    pub fn new(sample_rate: f32) -> Self {
        let cap = (sample_rate * 0.05) as usize + 16; // 50 ms ring
        Self {
            buffer: vec![0.0; cap],
            write: 0,
            cap,
            sample_rate: sample_rate.max(1.0),
            phase: 0.0,
            rate_hz: 0.6,
            depth_ms: 3.0,
            base_ms: 15.0,
            mix: 0.3,
            enabled: false,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    pub fn set_rate_hz(&mut self, hz: f32) {
        self.rate_hz = hz.clamp(0.01, 8.0);
    }

    pub fn set_depth_ms(&mut self, ms: f32) {
        self.depth_ms = ms.clamp(0.0, 10.0);
    }

    pub fn process(&mut self, x: f32) -> f32 {
        if !self.enabled {
            return x;
        }
        self.buffer[self.write] = x;
        let phase_step = self.rate_hz / self.sample_rate;
        self.phase = (self.phase + phase_step).rem_euclid(1.0);
        let lfo = (2.0 * PI * self.phase).sin();
        let delay_ms = self.base_ms + lfo * self.depth_ms;
        let delay_samples = (delay_ms * 0.001) * self.sample_rate;
        let read_f =
            (self.write as f32 + self.cap as f32 - delay_samples).rem_euclid(self.cap as f32);
        let read0 = read_f.floor() as usize % self.cap;
        let read1 = (read0 + 1) % self.cap;
        let frac = read_f - read_f.floor();
        let wet = self.buffer[read0] * (1.0 - frac) + self.buffer[read1] * frac;
        self.write = (self.write + 1) % self.cap;
        x * (1.0 - self.mix) + wet * self.mix
    }
}

/// A simple feedback delay — single-tap, tempo-agnostic. Default
/// 250 ms feedback at 0.35.
pub struct SimpleDelay {
    buffer: Vec<f32>,
    write: usize,
    cap: usize,
    delay_samples: usize,
    feedback: f32,
    mix: f32,
    enabled: bool,
}

impl SimpleDelay {
    pub fn new(sample_rate: f32) -> Self {
        let cap = (sample_rate * 2.0) as usize + 16; // 2 s ring
        let delay_samples = (sample_rate * 0.25) as usize;
        Self {
            buffer: vec![0.0; cap],
            write: 0,
            cap,
            delay_samples,
            feedback: 0.35,
            mix: 0.25,
            enabled: false,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    pub fn set_time_ms(&mut self, ms: f32) {
        let n = ((ms * 0.001) * self.cap as f32 / 2.0) as usize;
        self.delay_samples = n.clamp(1, self.cap - 1);
    }

    pub fn set_feedback(&mut self, fb: f32) {
        self.feedback = fb.clamp(0.0, 0.95);
    }

    pub fn process(&mut self, x: f32) -> f32 {
        if !self.enabled {
            return x;
        }
        let read = (self.write + self.cap - self.delay_samples) % self.cap;
        let delayed = self.buffer[read];
        self.buffer[self.write] = x + delayed * self.feedback;
        self.write = (self.write + 1) % self.cap;
        x * (1.0 - self.mix) + delayed * self.mix
    }
}

/// A lightweight comb-based reverb — three parallel comb filters
/// summed together. Tuned to give a ~1.5 s decay with a small
/// mix knob.
pub struct SimpleReverb {
    combs: Vec<Comb>,
    mix: f32,
    enabled: bool,
}

struct Comb {
    buffer: Vec<f32>,
    write: usize,
    cap: usize,
    feedback: f32,
    lp_state: f32,
    damp: f32,
}

impl Comb {
    fn new(sample_rate: f32, delay_ms: f32) -> Self {
        let cap = ((delay_ms * 0.001) * sample_rate) as usize + 1;
        let cap = cap.max(2);
        Self {
            buffer: vec![0.0; cap],
            write: 0,
            cap,
            feedback: 0.85,
            lp_state: 0.0,
            damp: 0.2,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let read = self.write;
        let out = self.buffer[read];
        self.lp_state = self.lp_state * self.damp + out * (1.0 - self.damp);
        self.buffer[self.write] = x + self.lp_state * self.feedback;
        self.write = (self.write + 1) % self.cap;
        out
    }
}

impl SimpleReverb {
    pub fn new(sample_rate: f32) -> Self {
        let sizes = [29.7, 37.1, 41.1];
        let combs = sizes.iter().map(|ms| Comb::new(sample_rate, *ms)).collect();
        Self {
            combs,
            mix: 0.2,
            enabled: false,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    pub fn process(&mut self, x: f32) -> f32 {
        if !self.enabled {
            return x;
        }
        let mut wet = 0.0_f32;
        for c in self.combs.iter_mut() {
            wet += c.process(x);
        }
        wet *= 1.0 / self.combs.len() as f32;
        x * (1.0 - self.mix) + wet * self.mix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * PI * freq * i as f32 / sr).sin())
            .collect()
    }

    #[test]
    fn drive_amount_zero_is_identity() {
        let d = FilterDrive::new();
        for s in [0.0, 0.5, -0.3, 0.9] {
            assert_eq!(d.process(s), s);
        }
    }

    #[test]
    fn drive_nonzero_adds_harmonics() {
        let mut d = FilterDrive::new();
        d.set_amount(0.8);
        // Peak stays bounded due to tanh.
        assert!(d.process(10.0) <= 1.0);
        assert!(d.process(-10.0) >= -1.0);
        // Drive should bend the sine noticeably.
        let input = sine(440.0, 44_100.0, 128, 0.5);
        let output: Vec<f32> = input.iter().map(|&x| d.process(x)).collect();
        let diff: f32 = input
            .iter()
            .zip(output.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(diff > 1.0, "drive didn't change signal, diff {}", diff);
    }

    #[test]
    fn key_tracking_doubles_cutoff_per_octave_at_full_amount() {
        let base = 1_000.0;
        let ref_note = 69.0;
        assert!((key_tracked_cutoff(base, ref_note, 1.0) - base).abs() < 1.0);
        assert!((key_tracked_cutoff(base, ref_note + 12.0, 1.0) - base * 2.0).abs() < 2.0);
        assert!((key_tracked_cutoff(base, ref_note - 12.0, 1.0) - base / 2.0).abs() < 2.0);
    }

    #[test]
    fn key_tracking_zero_amount_is_flat() {
        let base = 2_000.0;
        assert!((key_tracked_cutoff(base, 40.0, 0.0) - base).abs() < 1e-3);
        assert!((key_tracked_cutoff(base, 100.0, 0.0) - base).abs() < 1e-3);
    }

    #[test]
    fn master_strip_equal_power_pan() {
        let mut m = MasterStrip::new();
        m.set_volume_db(0.0);
        m.set_pan(0.0);
        let (l, r) = m.process_mono(1.0);
        // Center pan = -3 dB each side (equal power).
        assert!((l - r).abs() < 1e-4);
        assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
        m.set_pan(-1.0);
        let (l2, r2) = m.process_mono(1.0);
        assert!(l2 > 0.99 && r2.abs() < 1e-3);
        m.set_pan(1.0);
        let (l3, r3) = m.process_mono(1.0);
        assert!(r3 > 0.99 && l3.abs() < 1e-3);
    }

    #[test]
    fn master_volume_db_scales_output() {
        let mut m = MasterStrip::new();
        m.set_volume_db(-6.0);
        let (l, _) = m.process_mono(1.0);
        // -6 dB ≈ 0.501 × at center pan (0.501 × 0.7071 ≈ 0.354).
        assert!((l - 0.354).abs() < 0.02, "left at -6 dB center = {}", l);
    }

    #[test]
    fn chorus_disabled_is_passthrough() {
        let mut c = SimpleChorus::new(48_000.0);
        let mut output = Vec::new();
        for s in sine(440.0, 48_000.0, 1024, 0.5) {
            output.push(c.process(s));
        }
        // Compared to input, should be identical when disabled.
        let input = sine(440.0, 48_000.0, 1024, 0.5);
        for (a, b) in input.iter().zip(output.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn chorus_enabled_changes_signal() {
        let mut c = SimpleChorus::new(48_000.0);
        c.set_enabled(true);
        c.set_mix(0.5);
        let input = sine(440.0, 48_000.0, 4096, 0.5);
        let mut total_diff = 0.0_f32;
        for (i, s) in input.iter().enumerate() {
            let out = c.process(*s);
            total_diff += (out - s).abs();
            if i < 512 {
                continue;
            }
        }
        assert!(total_diff > 50.0, "chorus didn't alter signal");
    }

    #[test]
    fn delay_disabled_is_passthrough() {
        let mut d = SimpleDelay::new(48_000.0);
        let x = 0.4;
        assert_eq!(d.process(x), x);
    }

    #[test]
    fn delay_enabled_produces_echo() {
        let mut d = SimpleDelay::new(48_000.0);
        d.set_enabled(true);
        d.set_time_ms(10.0);
        d.set_mix(1.0); // pure wet so identity fails
        d.set_feedback(0.0);
        d.process(1.0);
        for _ in 0..479 {
            d.process(0.0);
        }
        // At ~10 ms = 480 samples the echo should land.
        let echo = d.process(0.0);
        assert!(echo.abs() > 0.1, "no echo found, got {}", echo);
    }

    #[test]
    fn reverb_disabled_is_passthrough() {
        let mut r = SimpleReverb::new(48_000.0);
        assert_eq!(r.process(0.7), 0.7);
    }

    #[test]
    fn reverb_tail_persists_after_input_stops() {
        let mut r = SimpleReverb::new(48_000.0);
        r.set_enabled(true);
        r.set_mix(1.0);
        r.process(1.0);
        // Accumulate the full tail over 3000 samples so we're not
        // dependent on hitting exactly one comb echo.
        let mut total_energy = 0.0_f32;
        for _ in 0..3_000 {
            total_energy += r.process(0.0).powi(2);
        }
        assert!(
            total_energy > 1e-3,
            "reverb tail has no energy: {}",
            total_energy
        );
    }

    #[test]
    fn built_in_fx_chain_pass_through_when_disabled() {
        let mut fx = SynthBuiltInFx::new(48_000.0);
        // All default-disabled → pure passthrough.
        for s in [0.1, 0.5, -0.3] {
            assert_eq!(fx.process(s), s);
        }
    }
}
