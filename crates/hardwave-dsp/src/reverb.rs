//! Algorithmic reverb: a Schroeder / Freeverb-style design with eight
//! parallel damped comb filters and four series allpass diffusers per
//! channel. Produces a smooth, dense tail; the core of the Reverb plug-in.
//!
//! "Size" sets the comb delay lengths (a bigger room has its echoes
//! further apart), "decay" sets each comb's feedback so the tail falls
//! 60 dB in that many seconds whatever the size, and "damping" rolls off
//! high frequencies inside the comb feedback, so highs die away sooner.
//! The wet signal is scaled so steady noise comes out about as loud as it
//! went in, at any size, decay and damping, and the mix is an equal-power
//! blend.

use crate::biquad::{Biquad, BiquadKind};

/// Freeverb's comb and allpass lengths, in samples at 44.1 kHz. They are
/// scaled to the sample rate and, for the combs, to the room size.
const COMB_TUNING: [f32; 8] = [
    1116.0, 1188.0, 1277.0, 1356.0, 1422.0, 1491.0, 1557.0, 1617.0,
];
const ALLPASS_TUNING: [f32; 4] = [556.0, 441.0, 341.0, 225.0];
/// The right channel's lines are this much longer, for a wide image.
const STEREO_SPREAD: f32 = 23.0;
const TUNING_RATE: f32 = 44_100.0;
/// Room size 0..1 scales the comb lengths from half to one and a half.
const SIZE_SCALE_MIN: f32 = 0.5;
const SIZE_SCALE_RANGE: f32 = 1.0;
/// The allpass diffusers' coefficient.
const ALLPASS_GAIN: f32 = 0.5;
/// Damping 0..1 sets the feedback low-pass coefficient up to this.
const DAMP_SCALE: f32 = 0.4;
/// Frequencies the wet level is averaged over, from 0 to half the rate.
const LEVEL_BANDS: usize = 32;

struct Comb {
    buffer: Vec<f32>,
    write_pos: usize,
    length: usize,
    feedback: f32,
    damp_lp_state: f32,
    damp: f32,
}

impl Comb {
    /// A comb that can be up to `capacity` samples long.
    fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0.0; capacity.max(2)],
            write_pos: 0,
            length: 1,
            feedback: 0.5,
            damp_lp_state: 0.0,
            damp: 0.0,
        }
    }

    fn set_length(&mut self, length: usize) {
        self.length = length.clamp(1, self.buffer.len() - 1);
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.damp_lp_state = 0.0;
        self.write_pos = 0;
    }

    fn process(&mut self, input: f32) -> f32 {
        let cap = self.buffer.len();
        let read_pos = (self.write_pos + cap - self.length) % cap;
        let out = self.buffer[read_pos];
        // 1-pole LP on the feedback path for damping.
        self.damp_lp_state = out * (1.0 - self.damp) + self.damp_lp_state * self.damp;
        self.buffer[self.write_pos] = input + self.damp_lp_state * self.feedback;
        self.write_pos = (self.write_pos + 1) % cap;
        out
    }
}

/// A Schroeder allpass: every frequency comes out at the level it went
/// in, only smeared in time. Freeverb's own "allpass" is not one (it
/// lifts some frequencies by up to 4.4 dB), which made the wet level
/// hard to predict.
struct Allpass {
    buffer: Vec<f32>,
    write_pos: usize,
}

impl Allpass {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; length.max(1)],
            write_pos: 0,
        }
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }

    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.buffer[self.write_pos];
        let v = input + delayed * ALLPASS_GAIN;
        self.buffer[self.write_pos] = v;
        self.write_pos = (self.write_pos + 1) % self.buffer.len();
        delayed - v * ALLPASS_GAIN
    }
}

/// Algorithmic reverb tail with parallel comb + series allpass.
/// Stereo is built by spreading one set of line lengths for L and a
/// slightly longer set for R to create a natural stereo image.
pub struct AlgorithmicReverb {
    combs_l: Vec<Comb>,
    combs_r: Vec<Comb>,
    allpass_l: Vec<Allpass>,
    allpass_r: Vec<Allpass>,
    room_size: f32,
    decay_secs: f32,
    /// The combs' feedback low-pass coefficient.
    damp: f32,
    pre_delay_l: Vec<f32>,
    pre_delay_r: Vec<f32>,
    pre_delay_pos: usize,
    pre_delay_samples: usize,
    dry_gain: f32,
    wet_gain: f32,
    /// What the combs' sum is scaled by so the tail is as loud as the
    /// input, worked out from the feedback whenever size or decay move.
    level: f32,
    tail_eq: bool,
    tail_filter_hp_l: Biquad,
    tail_filter_hp_r: Biquad,
    tail_filter_lp_l: Biquad,
    tail_filter_lp_r: Biquad,
    sample_rate: f32,
}

impl AlgorithmicReverb {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let rate = sample_rate / TUNING_RATE;
        let longest = (COMB_TUNING[7] + STEREO_SPREAD) * rate * (SIZE_SCALE_MIN + SIZE_SCALE_RANGE);
        let comb_capacity = longest.ceil() as usize + 2;
        let combs = || {
            (0..COMB_TUNING.len())
                .map(|_| Comb::new(comb_capacity))
                .collect()
        };
        let allpasses = |spread: f32| {
            ALLPASS_TUNING
                .iter()
                .map(|&len| Allpass::new(((len + spread) * rate).round() as usize))
                .collect()
        };
        let mut r = Self {
            combs_l: combs(),
            combs_r: combs(),
            allpass_l: allpasses(0.0),
            allpass_r: allpasses(STEREO_SPREAD),
            room_size: 0.5,
            decay_secs: 2.0,
            damp: 0.0,
            pre_delay_l: vec![0.0; sample_rate as usize],
            pre_delay_r: vec![0.0; sample_rate as usize],
            pre_delay_pos: 0,
            pre_delay_samples: 0,
            dry_gain: 1.0,
            wet_gain: 0.0,
            level: 1.0,
            tail_eq: false,
            tail_filter_hp_l: Biquad::default(),
            tail_filter_hp_r: Biquad::default(),
            tail_filter_lp_l: Biquad::default(),
            tail_filter_lp_r: Biquad::default(),
            sample_rate,
        };
        r.set_mix(0.3);
        r.set_damping(0.5);
        r
    }

    /// The combs' lengths for the room size, their damping, and their
    /// feedback for the decay: a comb `len` samples long loses 60 dB over
    /// the decay time when each pass takes `-60 * len / (rate * decay)` dB
    /// off. Then the wet level that keeps steady noise at the level it
    /// came in: at a frequency where a pass through the comb keeps `a` of
    /// the level, the comb has `1 / (1 - a^2)` times its input's energy,
    /// averaged here over the band, since damping lowers `a` in the highs.
    fn retune(&mut self) {
        let scale =
            (SIZE_SCALE_MIN + SIZE_SCALE_RANGE * self.room_size) * self.sample_rate / TUNING_RATE;
        let decay = self.decay_secs.max(0.01);
        let d = self.damp;
        // The damping low-pass's power gain in each band.
        let mut lowpass = [0.0_f32; LEVEL_BANDS];
        for (k, h) in lowpass.iter_mut().enumerate() {
            let w = std::f32::consts::PI * (k as f32 + 0.5) / LEVEL_BANDS as f32;
            *h = (1.0 - d) * (1.0 - d) / (1.0 - 2.0 * d * w.cos() + d * d);
        }
        let mut energy = 0.0_f32;
        for (combs, spread) in [(&mut self.combs_l, 0.0), (&mut self.combs_r, STEREO_SPREAD)] {
            for (c, &tuning) in combs.iter_mut().zip(COMB_TUNING.iter()) {
                c.set_length(((tuning + spread) * scale).round() as usize);
                let g = 10f32.powf(-3.0 * c.length as f32 / (self.sample_rate * decay));
                c.feedback = g.min(0.9999);
                c.damp = d;
                let g2 = c.feedback * c.feedback;
                energy +=
                    lowpass.iter().map(|&h| 1.0 / (1.0 - g2 * h)).sum::<f32>() / LEVEL_BANDS as f32;
            }
        }
        // Both channels were summed: half of it is one channel's.
        self.level = (2.0 / energy).sqrt();
    }

    /// Room size in `[0, 1]`: how far apart the echoes are. The decay
    /// time stays as set.
    pub fn set_room_size(&mut self, room_size: f32) {
        self.room_size = room_size.clamp(0.0, 1.0);
        self.retune();
    }

    /// Damping in `[0, 1]`. Higher values roll off high frequencies
    /// faster in each comb's feedback, producing a warmer tail.
    pub fn set_damping(&mut self, damping: f32) {
        self.damp = damping.clamp(0.0, 1.0) * DAMP_SCALE;
        self.retune();
    }

    /// How long the tail takes to fall by 60 dB, in seconds.
    pub fn set_decay_time(&mut self, decay_secs: f32) {
        self.decay_secs = decay_secs.clamp(0.05, 30.0);
        self.retune();
    }

    /// Pre-delay in milliseconds before the reverb tail starts.
    pub fn set_pre_delay_ms(&mut self, ms: f32) {
        let samples = ((ms.max(0.0) / 1000.0) * self.sample_rate) as usize;
        self.pre_delay_samples = samples.min(self.pre_delay_l.len().saturating_sub(1));
    }

    /// Dry/wet mix. 0 = all dry, 1 = all wet; an equal-power blend in
    /// between, so a mix keeps about the dry signal's loudness.
    pub fn set_mix(&mut self, mix: f32) {
        let m = mix.clamp(0.0, 1.0);
        let angle = m * std::f32::consts::FRAC_PI_2;
        self.dry_gain = if m >= 1.0 { 0.0 } else { angle.cos() };
        self.wet_gain = if m <= 0.0 { 0.0 } else { angle.sin() };
    }

    /// Tail EQ: low-cut (high-pass) and high-cut (low-pass) shaping
    /// on the wet signal only. Until it is set the tail is not filtered.
    pub fn set_tail_eq(&mut self, low_cut_hz: f32, high_cut_hz: f32) {
        let sr = self.sample_rate;
        for f in [&mut self.tail_filter_hp_l, &mut self.tail_filter_hp_r] {
            f.set(BiquadKind::HighPass, sr, low_cut_hz, 0.707, 0.0);
        }
        for f in [&mut self.tail_filter_lp_l, &mut self.tail_filter_lp_r] {
            f.set(BiquadKind::LowPass, sr, high_cut_hz, 0.707, 0.0);
        }
        self.tail_eq = true;
    }

    pub fn reset(&mut self) {
        for c in self.combs_l.iter_mut().chain(self.combs_r.iter_mut()) {
            c.reset();
        }
        for a in self.allpass_l.iter_mut().chain(self.allpass_r.iter_mut()) {
            a.reset();
        }
        self.pre_delay_l.fill(0.0);
        self.pre_delay_r.fill(0.0);
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
        let mut wet_l = comb_sum_l * self.level;
        let mut wet_r = comb_sum_r * self.level;
        for a in self.allpass_l.iter_mut() {
            wet_l = a.process(wet_l);
        }
        for a in self.allpass_r.iter_mut() {
            wet_r = a.process(wet_r);
        }

        // Tail EQ. A filter that was never set has all-zero coefficients
        // and silenced the whole wet signal: the plug-in never set it.
        if self.tail_eq {
            wet_l = self
                .tail_filter_lp_l
                .process_mono(self.tail_filter_hp_l.process_mono(wet_l));
            wet_r = self
                .tail_filter_lp_r
                .process_mono(self.tail_filter_hp_r.process_mono(wet_r));
        }

        (
            dry_l * self.dry_gain + wet_l * self.wet_gain,
            dry_r * self.dry_gain + wet_r * self.wet_gain,
        )
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
        // Feed an impulse; expect a tail that is still there half a
        // second later and quieter than its first tenth of a second.
        let mut energy_early = 0.0_f32;
        let mut energy_late = 0.0_f32;
        for t in 0..48_000 {
            let x = if t == 0 { 1.0 } else { 0.0 };
            let (l, _r) = r.process(x, x);
            if (0..4_800).contains(&t) {
                energy_early += l * l;
            }
            if (24_000..28_800).contains(&t) {
                energy_late += l * l;
            }
        }
        assert!(
            energy_early > 0.0,
            "expected tail energy shortly after impulse"
        );
        // The default 2 s decay: still ringing, and decaying.
        assert!(
            energy_late > 0.0 && energy_late < energy_early,
            "late {energy_late} early {energy_early}"
        );
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
    fn reverb_tail_grows_with_decay_time() {
        fn tail_energy(decay_secs: f32) -> f32 {
            let mut r = AlgorithmicReverb::new(48_000.0);
            r.set_decay_time(decay_secs);
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
        let small = tail_energy(0.5);
        let large = tail_energy(4.0);
        assert!(
            large > small,
            "a longer decay should have more tail: {large} vs {small}"
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
