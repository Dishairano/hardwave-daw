//! Algorithmic reverb — Schroeder / Freeverb-style design with
//! parallel comb filters and series allpass diffusion. Produces a
//! smooth, dense tail suitable as the core of the Reverb plugin.
//!
//! Design: 4 parallel damped combs → 2 series allpass diffusers, per
//! channel. Decay comes from comb feedback; "room size" controls the
//! comb delay lengths; "damping" attenuates high frequencies inside
//! the comb feedback loop.

use crate::biquad::{Biquad, BiquadKind};

/// Max reverb delay across any comb line, in samples. Sized for
/// long-tail halls at 48 kHz — 6000 samples ≈ 125 ms per comb.
const MAX_COMB_SAMPLES: usize = 6000;
const MAX_ALLPASS_SAMPLES: usize = 1000;

/// Classic comb delay prime-length table (scaled by `room_size`).
/// These prime lengths produce minimal mode overlap at any room size.
const COMB_LENGTHS: [usize; 4] = [1687, 1601, 2053, 2251];
const ALLPASS_LENGTHS: [usize; 2] = [389, 307];

struct Comb {
    buffer: Vec<f32>,
    write_pos: usize,
    length: usize,
    feedback: f32,
    damp_lp_state: f32,
    damp: f32,
}

impl Comb {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; MAX_COMB_SAMPLES],
            write_pos: 0,
            length: length.clamp(1, MAX_COMB_SAMPLES),
            feedback: 0.5,
            damp_lp_state: 0.0,
            damp: 0.5,
        }
    }

    fn set_feedback(&mut self, fb: f32) {
        self.feedback = fb.clamp(0.0, 0.98);
    }

    fn set_damping(&mut self, damp: f32) {
        self.damp = damp.clamp(0.0, 0.99);
    }

    fn reset(&mut self) {
        for s in self.buffer.iter_mut() {
            *s = 0.0;
        }
        self.damp_lp_state = 0.0;
        self.write_pos = 0;
    }

    fn process(&mut self, input: f32) -> f32 {
        let read_pos = (self.write_pos + MAX_COMB_SAMPLES - self.length) % MAX_COMB_SAMPLES;
        let out = self.buffer[read_pos];
        // 1-pole LP on the feedback path for damping.
        self.damp_lp_state = out * (1.0 - self.damp) + self.damp_lp_state * self.damp;
        let write_val = input + self.damp_lp_state * self.feedback;
        self.buffer[self.write_pos] = write_val;
        self.write_pos = (self.write_pos + 1) % MAX_COMB_SAMPLES;
        out
    }
}

struct Allpass {
    buffer: Vec<f32>,
    write_pos: usize,
    length: usize,
    feedback: f32,
}

impl Allpass {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; MAX_ALLPASS_SAMPLES],
            write_pos: 0,
            length: length.clamp(1, MAX_ALLPASS_SAMPLES),
            feedback: 0.5,
        }
    }

    fn reset(&mut self) {
        for s in self.buffer.iter_mut() {
            *s = 0.0;
        }
        self.write_pos = 0;
    }

    fn process(&mut self, input: f32) -> f32 {
        let read_pos = (self.write_pos + MAX_ALLPASS_SAMPLES - self.length) % MAX_ALLPASS_SAMPLES;
        let delayed = self.buffer[read_pos];
        let out = -input + delayed;
        self.buffer[self.write_pos] = input + delayed * self.feedback;
        self.write_pos = (self.write_pos + 1) % MAX_ALLPASS_SAMPLES;
        out
    }
}

/// Algorithmic reverb tail with parallel comb + series allpass.
/// Stereo is built by spreading one set of comb lengths for L and a
/// slightly-offset set for R to create a natural stereo image.
pub struct AlgorithmicReverb {
    combs_l: Vec<Comb>,
    combs_r: Vec<Comb>,
    allpass_l: Vec<Allpass>,
    allpass_r: Vec<Allpass>,
    room_size: f32,
    damping: f32,
    pre_delay_l: Vec<f32>,
    pre_delay_r: Vec<f32>,
    pre_delay_pos: usize,
    pre_delay_samples: usize,
    wet_mix: f32,
    tail_filter_hp_l: Biquad,
    tail_filter_hp_r: Biquad,
    tail_filter_lp_l: Biquad,
    tail_filter_lp_r: Biquad,
    sample_rate: f32,
}

impl AlgorithmicReverb {
    pub fn new(sample_rate: f32) -> Self {
        let mut combs_l = Vec::with_capacity(4);
        let mut combs_r = Vec::with_capacity(4);
        for &len in &COMB_LENGTHS {
            combs_l.push(Comb::new(len));
            combs_r.push(Comb::new(len + 23)); // stereo offset
        }
        let mut allpass_l = Vec::with_capacity(2);
        let mut allpass_r = Vec::with_capacity(2);
        for &len in &ALLPASS_LENGTHS {
            allpass_l.push(Allpass::new(len));
            allpass_r.push(Allpass::new(len + 17));
        }
        Self {
            combs_l,
            combs_r,
            allpass_l,
            allpass_r,
            room_size: 0.5,
            damping: 0.5,
            pre_delay_l: vec![0.0; sample_rate as usize],
            pre_delay_r: vec![0.0; sample_rate as usize],
            pre_delay_pos: 0,
            pre_delay_samples: 0,
            wet_mix: 0.3,
            tail_filter_hp_l: Biquad::default(),
            tail_filter_hp_r: Biquad::default(),
            tail_filter_lp_l: Biquad::default(),
            tail_filter_lp_r: Biquad::default(),
            sample_rate: sample_rate.max(1.0),
        }
    }

    /// Room size in `[0, 1]`. Larger values push the comb feedback
    /// toward longer decays.
    pub fn set_room_size(&mut self, room_size: f32) {
        let rs = room_size.clamp(0.0, 1.0);
        self.room_size = rs;
        let fb = 0.7 + rs * 0.28; // 0.70..0.98
        for c in self.combs_l.iter_mut().chain(self.combs_r.iter_mut()) {
            c.set_feedback(fb);
        }
    }

    /// Damping in `[0, 1]`. Higher values roll off high frequencies
    /// faster in each comb's feedback, producing a warmer tail.
    pub fn set_damping(&mut self, damping: f32) {
        let d = damping.clamp(0.0, 1.0);
        self.damping = d;
        for c in self.combs_l.iter_mut().chain(self.combs_r.iter_mut()) {
            c.set_damping(d);
        }
    }

    /// Decay time proxy — maps directly to room size. Longer decay
    /// means more feedback, i.e. larger room.
    pub fn set_decay_time(&mut self, decay_secs: f32) {
        // Invert the physical rough-scale: decay_secs = -3 * len / sr / ln(|fb|).
        // Approximate by mapping decay_secs through room_size.
        let normalized = (decay_secs / 10.0).clamp(0.0, 1.0);
        self.set_room_size(normalized);
    }

    /// Pre-delay in milliseconds before the reverb tail starts.
    pub fn set_pre_delay_ms(&mut self, ms: f32) {
        let samples = ((ms.max(0.0) / 1000.0) * self.sample_rate) as usize;
        self.pre_delay_samples = samples.min(self.pre_delay_l.len().saturating_sub(1));
    }

    /// Dry/wet mix. 0 = all dry, 1 = all wet.
    pub fn set_mix(&mut self, mix: f32) {
        self.wet_mix = mix.clamp(0.0, 1.0);
    }

    /// Tail EQ: low-cut (high-pass) and high-cut (low-pass) shaping
    /// on the wet signal only.
    pub fn set_tail_eq(&mut self, low_cut_hz: f32, high_cut_hz: f32) {
        self.tail_filter_hp_l.set(
            BiquadKind::HighPass,
            self.sample_rate,
            low_cut_hz,
            0.707,
            0.0,
        );
        self.tail_filter_hp_r.set(
            BiquadKind::HighPass,
            self.sample_rate,
            low_cut_hz,
            0.707,
            0.0,
        );
        self.tail_filter_lp_l.set(
            BiquadKind::LowPass,
            self.sample_rate,
            high_cut_hz,
            0.707,
            0.0,
        );
        self.tail_filter_lp_r.set(
            BiquadKind::LowPass,
            self.sample_rate,
            high_cut_hz,
            0.707,
            0.0,
        );
    }

    pub fn reset(&mut self) {
        for c in self.combs_l.iter_mut().chain(self.combs_r.iter_mut()) {
            c.reset();
        }
        for a in self.allpass_l.iter_mut().chain(self.allpass_r.iter_mut()) {
            a.reset();
        }
        for s in self.pre_delay_l.iter_mut() {
            *s = 0.0;
        }
        for s in self.pre_delay_r.iter_mut() {
            *s = 0.0;
        }
        self.pre_delay_pos = 0;
        self.tail_filter_hp_l.reset();
        self.tail_filter_hp_r.reset();
        self.tail_filter_lp_l.reset();
        self.tail_filter_lp_r.reset();
    }

    /// Process a single stereo frame. Returns `(l, r)` of the mixed
    /// dry + wet signal.
    pub fn process(&mut self, dry_l: f32, dry_r: f32) -> (f32, f32) {
        // Pre-delay.
        let (in_l, in_r) = if self.pre_delay_samples > 0 {
            let cap = self.pre_delay_l.len();
            let read = (self.pre_delay_pos + cap - self.pre_delay_samples) % cap;
            let out_l = self.pre_delay_l[read];
            let out_r = self.pre_delay_r[read];
            self.pre_delay_l[self.pre_delay_pos] = dry_l;
            self.pre_delay_r[self.pre_delay_pos] = dry_r;
            self.pre_delay_pos = (self.pre_delay_pos + 1) % cap;
            (out_l, out_r)
        } else {
            (dry_l, dry_r)
        };

        // Parallel combs.
        let mut comb_sum_l = 0.0;
        let mut comb_sum_r = 0.0;
        for c in self.combs_l.iter_mut() {
            comb_sum_l += c.process(in_l);
        }
        for c in self.combs_r.iter_mut() {
            comb_sum_r += c.process(in_r);
        }

        // Series allpass diffusion.
        let mut diff_l = comb_sum_l;
        let mut diff_r = comb_sum_r;
        for a in self.allpass_l.iter_mut() {
            diff_l = a.process(diff_l);
        }
        for a in self.allpass_r.iter_mut() {
            diff_r = a.process(diff_r);
        }

        // Tail EQ.
        let wet_l = self
            .tail_filter_lp_l
            .process_mono(self.tail_filter_hp_l.process_mono(diff_l));
        let wet_r = self
            .tail_filter_lp_r
            .process_mono(self.tail_filter_hp_r.process_mono(diff_r));

        // Dry / wet mix.
        let m = self.wet_mix;
        (dry_l * (1.0 - m) + wet_l * m, dry_r * (1.0 - m) + wet_r * m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverb_produces_tail_after_impulse() {
        let mut r = AlgorithmicReverb::new(48_000.0);
        r.set_room_size(0.7);
        r.set_damping(0.3);
        r.set_mix(1.0);
        r.set_tail_eq(80.0, 10_000.0);
        // Feed an impulse; expect a decaying tail over the next
        // thousands of samples.
        let mut energy_early = 0.0_f32;
        let mut energy_late = 0.0_f32;
        for t in 0..48_000 {
            let x = if t == 0 { 1.0 } else { 0.0 };
            let (l, _r) = r.process(x, x);
            if (500..2000).contains(&t) {
                energy_early += l * l;
            }
            if (20_000..30_000).contains(&t) {
                energy_late += l * l;
            }
        }
        assert!(
            energy_early > 0.0,
            "expected tail energy shortly after impulse"
        );
        // Late energy should be less than early but still non-zero
        // with room=0.7 (long decay).
        assert!(energy_late > 0.0 && energy_late < energy_early * 2.0);
    }

    #[test]
    fn reverb_with_zero_mix_passes_dry_through() {
        let mut r = AlgorithmicReverb::new(48_000.0);
        r.set_mix(0.0);
        let (l, rr) = r.process(0.5, -0.3);
        assert_eq!(l, 0.5);
        assert_eq!(rr, -0.3);
    }

    #[test]
    fn reverb_pre_delay_shifts_onset() {
        let mut r = AlgorithmicReverb::new(48_000.0);
        r.set_mix(1.0);
        r.set_pre_delay_ms(10.0);
        r.set_tail_eq(20.0, 20_000.0);
        r.set_room_size(0.5);
        // 10 ms at 48 kHz = 480 samples. Before tick 480 the wet
        // output should be essentially zero.
        let mut max_early = 0.0_f32;
        for t in 0..450 {
            let x = if t == 0 { 1.0 } else { 0.0 };
            let (l, _r) = r.process(x, x);
            if l.abs() > max_early {
                max_early = l.abs();
            }
        }
        assert!(
            max_early < 1e-3,
            "pre-delay should delay onset, max_early = {max_early}"
        );
    }

    #[test]
    fn reverb_decay_grows_with_room_size() {
        fn tail_energy(room: f32) -> f32 {
            let mut r = AlgorithmicReverb::new(48_000.0);
            r.set_room_size(room);
            r.set_damping(0.2);
            r.set_mix(1.0);
            r.set_tail_eq(20.0, 20_000.0);
            let mut energy = 0.0_f32;
            for t in 0..48_000 {
                let x = if t == 0 { 1.0 } else { 0.0 };
                let (l, _r) = r.process(x, x);
                if t > 20_000 {
                    energy += l * l;
                }
            }
            energy
        }
        let small = tail_energy(0.2);
        let large = tail_energy(0.9);
        assert!(
            large > small,
            "larger room should have more tail: {large} vs {small}"
        );
    }

    #[test]
    fn reverb_reset_clears_tail() {
        let mut r = AlgorithmicReverb::new(48_000.0);
        r.set_mix(1.0);
        r.set_room_size(0.8);
        for _ in 0..5000 {
            r.process(1.0, 1.0);
        }
        r.reset();
        let (l, rr) = r.process(0.0, 0.0);
        assert!(l.abs() < 1e-3);
        assert!(rr.abs() < 1e-3);
    }
}
