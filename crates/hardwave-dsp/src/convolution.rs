//! Convolution reverb — direct-form time-domain convolution of an
//! impulse response with an input signal. Suitable for short IRs;
//! an FFT-based partitioned convolution would scale better for long
//! IRs but this primitive covers the core feature set the roadmap
//! calls out (load/trim/pre-delay/EQ/low-CPU mode).

use crate::biquad::{Biquad, BiquadKind};

/// Max IR length in samples — 2 seconds at 48 kHz. Past this, the
/// direct convolution becomes expensive and callers should use
/// `set_low_cpu_mode` to truncate the IR further.
pub const MAX_IR_SAMPLES: usize = 96_000;

/// Single-channel convolution engine. Holds the IR and a ring buffer
/// of recent input samples. For stereo, instantiate two instances.
struct ConvolutionChannel {
    ir: Vec<f32>,
    input_ring: Vec<f32>,
    ring_pos: usize,
    effective_len: usize,
}

impl ConvolutionChannel {
    fn new() -> Self {
        Self {
            ir: Vec::new(),
            input_ring: vec![0.0; MAX_IR_SAMPLES],
            ring_pos: 0,
            effective_len: 0,
        }
    }

    fn load_ir(&mut self, samples: &[f32]) {
        let n = samples.len().min(MAX_IR_SAMPLES);
        self.ir.clear();
        self.ir.extend_from_slice(&samples[..n]);
        self.effective_len = n;
    }

    fn set_trim(&mut self, trim_samples: usize) {
        self.effective_len = trim_samples.min(self.ir.len());
    }

    fn reset(&mut self) {
        for s in self.input_ring.iter_mut() {
            *s = 0.0;
        }
        self.ring_pos = 0;
    }

    fn process(&mut self, input: f32) -> f32 {
        self.input_ring[self.ring_pos] = input;
        let len = self.effective_len;
        if len == 0 {
            self.ring_pos = (self.ring_pos + 1) % self.input_ring.len();
            return 0.0;
        }
        let ring_len = self.input_ring.len();
        let mut sum = 0.0_f32;
        for (i, &coef) in self.ir[..len].iter().enumerate() {
            let idx = (self.ring_pos + ring_len - i) % ring_len;
            sum += self.input_ring[idx] * coef;
        }
        self.ring_pos = (self.ring_pos + 1) % ring_len;
        sum
    }
}

/// Stereo convolution reverb with pre-delay, dry/wet mix, IR
/// length trim (low-CPU mode), and optional stereo width + tail EQ.
/// Convolution itself is direct-form (O(IR_len) per sample); for
/// long IRs this is CPU-heavy — the length-trim knob is the
/// low-CPU escape hatch.
pub struct ConvolutionReverb {
    left: ConvolutionChannel,
    right: ConvolutionChannel,
    pre_delay_l: Vec<f32>,
    pre_delay_r: Vec<f32>,
    pre_delay_pos: usize,
    pre_delay_samples: usize,
    wet_mix: f32,
    stereo_width: f32,
    tail_hp_l: Biquad,
    tail_hp_r: Biquad,
    tail_lp_l: Biquad,
    tail_lp_r: Biquad,
    sample_rate: f32,
}

impl ConvolutionReverb {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            left: ConvolutionChannel::new(),
            right: ConvolutionChannel::new(),
            pre_delay_l: vec![0.0; sample_rate as usize],
            pre_delay_r: vec![0.0; sample_rate as usize],
            pre_delay_pos: 0,
            pre_delay_samples: 0,
            wet_mix: 0.3,
            stereo_width: 1.0,
            tail_hp_l: Biquad::default(),
            tail_hp_r: Biquad::default(),
            tail_lp_l: Biquad::default(),
            tail_lp_r: Biquad::default(),
            sample_rate: sample_rate.max(1.0),
        }
    }

    /// Load a mono IR onto both channels. Use `load_ir_stereo` for
    /// true-stereo IRs with separate L/R responses.
    pub fn load_ir_mono(&mut self, samples: &[f32]) {
        self.left.load_ir(samples);
        self.right.load_ir(samples);
    }

    /// Load separate L/R impulse responses.
    pub fn load_ir_stereo(&mut self, left: &[f32], right: &[f32]) {
        self.left.load_ir(left);
        self.right.load_ir(right);
    }

    /// IR length trim in samples. Low-CPU mode is just a strong trim
    /// — e.g. `set_trim(sample_rate / 4)` clips the IR to 250 ms.
    pub fn set_trim(&mut self, trim_samples: usize) {
        self.left.set_trim(trim_samples);
        self.right.set_trim(trim_samples);
    }

    /// Low-CPU mode: truncate the IR to `fraction × loaded_length`.
    /// Range `[0.1, 1.0]`; lower values use less CPU at the cost of
    /// a shorter tail.
    pub fn set_low_cpu_mode(&mut self, fraction: f32) {
        let f = fraction.clamp(0.1, 1.0);
        let new_len_l = (self.left.ir.len() as f32 * f) as usize;
        let new_len_r = (self.right.ir.len() as f32 * f) as usize;
        self.left.set_trim(new_len_l);
        self.right.set_trim(new_len_r);
    }

    pub fn set_pre_delay_ms(&mut self, ms: f32) {
        let samples = ((ms.max(0.0) / 1000.0) * self.sample_rate) as usize;
        self.pre_delay_samples = samples.min(self.pre_delay_l.len().saturating_sub(1));
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.wet_mix = mix.clamp(0.0, 1.0);
    }

    pub fn set_stereo_width(&mut self, width: f32) {
        self.stereo_width = width.clamp(0.0, 2.0);
    }

    pub fn set_tail_eq(&mut self, low_cut_hz: f32, high_cut_hz: f32) {
        self.tail_hp_l.set(
            BiquadKind::HighPass,
            self.sample_rate,
            low_cut_hz,
            0.707,
            0.0,
        );
        self.tail_hp_r.set(
            BiquadKind::HighPass,
            self.sample_rate,
            low_cut_hz,
            0.707,
            0.0,
        );
        self.tail_lp_l.set(
            BiquadKind::LowPass,
            self.sample_rate,
            high_cut_hz,
            0.707,
            0.0,
        );
        self.tail_lp_r.set(
            BiquadKind::LowPass,
            self.sample_rate,
            high_cut_hz,
            0.707,
            0.0,
        );
    }

    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
        for s in self.pre_delay_l.iter_mut() {
            *s = 0.0;
        }
        for s in self.pre_delay_r.iter_mut() {
            *s = 0.0;
        }
        self.pre_delay_pos = 0;
        self.tail_hp_l.reset();
        self.tail_hp_r.reset();
        self.tail_lp_l.reset();
        self.tail_lp_r.reset();
    }

    pub fn process(&mut self, dry_l: f32, dry_r: f32) -> (f32, f32) {
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

        let wet_l_raw = self.left.process(in_l);
        let wet_r_raw = self.right.process(in_r);

        // Apply tail EQ.
        let wet_l_eq = self
            .tail_lp_l
            .process_mono(self.tail_hp_l.process_mono(wet_l_raw));
        let wet_r_eq = self
            .tail_lp_r
            .process_mono(self.tail_hp_r.process_mono(wet_r_raw));

        // Stereo width on the wet signal only via mid/side scaling.
        let mid = (wet_l_eq + wet_r_eq) * 0.5;
        let side = (wet_l_eq - wet_r_eq) * 0.5 * self.stereo_width;
        let wet_l = mid + side;
        let wet_r = mid - side;

        let m = self.wet_mix;
        (dry_l * (1.0 - m) + wet_l * m, dry_r * (1.0 - m) + wet_r * m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_in_impulse_out_returns_ir() {
        let mut c = ConvolutionReverb::new(48_000.0);
        let ir: Vec<f32> = (0..10).map(|i| i as f32 * 0.1).collect();
        c.load_ir_mono(&ir);
        c.set_mix(1.0);
        c.set_tail_eq(1.0, 24_000.0);
        c.set_stereo_width(1.0);
        // Feed an impulse; the wet output should be the IR itself.
        let mut out = Vec::new();
        for t in 0..20 {
            let x = if t == 0 { 1.0 } else { 0.0 };
            let (l, _r) = c.process(x, x);
            out.push(l);
        }
        // IR values should appear at each tick.
        for (i, &expected) in ir.iter().enumerate() {
            assert!(
                (out[i] - expected).abs() < 0.02,
                "tick {i}: got {} expected {expected}",
                out[i]
            );
        }
    }

    #[test]
    fn zero_mix_passes_dry_through() {
        let mut c = ConvolutionReverb::new(48_000.0);
        c.load_ir_mono(&[0.5, 0.5, 0.5]);
        c.set_mix(0.0);
        let (l, r) = c.process(0.42, -0.17);
        assert_eq!(l, 0.42);
        assert_eq!(r, -0.17);
    }

    #[test]
    fn trim_shortens_tail() {
        let mut full = ConvolutionReverb::new(48_000.0);
        let mut trimmed = ConvolutionReverb::new(48_000.0);
        let ir: Vec<f32> = (0..1000)
            .map(|i| 0.01 * (1.0 - i as f32 / 1000.0))
            .collect();
        full.load_ir_mono(&ir);
        trimmed.load_ir_mono(&ir);
        full.set_mix(1.0);
        trimmed.set_mix(1.0);
        trimmed.set_low_cpu_mode(0.25);
        full.set_tail_eq(1.0, 24_000.0);
        trimmed.set_tail_eq(1.0, 24_000.0);
        let mut full_energy = 0.0_f32;
        let mut trimmed_energy = 0.0_f32;
        for t in 0..2000 {
            let x = if t == 0 { 1.0 } else { 0.0 };
            let (lf, _) = full.process(x, x);
            let (lt, _) = trimmed.process(x, x);
            full_energy += lf * lf;
            trimmed_energy += lt * lt;
        }
        assert!(
            trimmed_energy < full_energy * 0.75,
            "trimmed tail should have less energy: {trimmed_energy} vs {full_energy}"
        );
    }

    #[test]
    fn pre_delay_shifts_onset() {
        let mut c = ConvolutionReverb::new(48_000.0);
        c.load_ir_mono(&[1.0]); // 1-sample IR
        c.set_mix(1.0);
        c.set_pre_delay_ms(5.0);
        c.set_tail_eq(1.0, 24_000.0);
        // 5 ms = 240 samples. Before tick 240 we should see no wet.
        let mut first_arrival = None;
        for t in 0..500 {
            let x = if t == 0 { 1.0 } else { 0.0 };
            let (l, _r) = c.process(x, x);
            if l.abs() > 0.5 && first_arrival.is_none() {
                first_arrival = Some(t);
            }
        }
        // Impulse at t=0 goes through 240-sample pre-delay then 1-sample
        // IR convolution, so arrival is around tick 240.
        assert!(
            first_arrival.is_some(),
            "pre-delayed impulse should arrive eventually"
        );
        let t = first_arrival.unwrap();
        assert!((235..=245).contains(&t), "arrival tick = {t}");
    }

    #[test]
    fn stereo_width_collapses_at_zero() {
        let mut c = ConvolutionReverb::new(48_000.0);
        c.load_ir_stereo(&[1.0, 0.0], &[-1.0, 0.0]);
        c.set_mix(1.0);
        c.set_stereo_width(0.0);
        c.set_tail_eq(1.0, 24_000.0);
        // width=0 should make L and R identical on the wet side.
        let (l, r) = c.process(1.0, 1.0);
        assert!(
            (l - r).abs() < 1e-6,
            "width=0 should collapse: L={l}, R={r}"
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut c = ConvolutionReverb::new(48_000.0);
        c.load_ir_mono(&[0.5, 0.3, 0.2]);
        c.set_mix(1.0);
        for _ in 0..100 {
            c.process(1.0, 1.0);
        }
        c.reset();
        let (l, r) = c.process(0.0, 0.0);
        assert_eq!(l, 0.0);
        assert_eq!(r, 0.0);
    }
}
