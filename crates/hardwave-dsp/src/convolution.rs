//! Convolution reverb: an impulse response convolved with the input.
//!
//! It was direct-form, every sample multiplied against the whole impulse:
//! the default 1.8 s plate is 86,000 multiply-adds per sample per channel,
//! about eight billion a second, which no processor plays in real time.
//! Putting the reverb on a track overloaded the audio thread.
//!
//! Now it is partitioned. The first `HEAD` taps stay direct, so nothing is
//! delayed; the rest of the impulse is cut into blocks of `HEAD` samples
//! whose spectra are kept, and each block of input is turned into a
//! spectrum once and multiplied with all of them (uniformly partitioned
//! overlap-save, with the input's spectra in a frequency-domain delay
//! line). That is about a hundred times less work. Every buffer and the
//! FFT plans are made when the reverb is made, so nothing allocates while
//! it plays.

use std::sync::Arc;

use rustfft::{num_complex::Complex32, Fft, FftPlanner};

use crate::biquad::{Biquad, BiquadKind};

/// Max IR length in samples — 2 seconds at 48 kHz. Longer impulses are
/// cut; `set_low_cpu_mode` shortens them further.
pub const MAX_IR_SAMPLES: usize = 96_000;

/// Taps convolved directly, and the partition size of the rest.
const HEAD: usize = 256;
/// The FFT size: two partitions, for overlap-save.
const FFT_N: usize = 2 * HEAD;
/// Bins kept of a real signal's spectrum (the rest mirror them).
const BINS: usize = HEAD + 1;
/// Partitions of the tail, for the longest impulse.
const MAX_PARTS: usize = (MAX_IR_SAMPLES - HEAD).div_ceil(HEAD);

/// Single-channel convolution engine. For stereo, instantiate two.
struct ConvolutionChannel {
    ir: Vec<f32>,
    effective_len: usize,
    /// The last `HEAD` input samples, for the direct head.
    head_ring: Vec<f32>,
    head_pos: usize,
    /// The tail's partitions as spectra, `BINS` each, and how many count.
    parts: Vec<Complex32>,
    active_parts: usize,
    /// The input blocks' spectra, newest at `fdl_pos`.
    fdl: Vec<Complex32>,
    fdl_pos: usize,
    /// The previous block and the one filling, time domain.
    in_block: Vec<f32>,
    in_fill: usize,
    /// The tail's output for the block now playing.
    out_block: Vec<f32>,
    work: Vec<Complex32>,
    scratch: Vec<Complex32>,
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
}

impl ConvolutionChannel {
    fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_N);
        let ifft = planner.plan_fft_inverse(FFT_N);
        let scratch_len = fft
            .get_inplace_scratch_len()
            .max(ifft.get_inplace_scratch_len());
        Self {
            ir: Vec::with_capacity(MAX_IR_SAMPLES),
            effective_len: 0,
            head_ring: vec![0.0; HEAD],
            head_pos: 0,
            parts: vec![Complex32::new(0.0, 0.0); MAX_PARTS * BINS],
            active_parts: 0,
            fdl: vec![Complex32::new(0.0, 0.0); MAX_PARTS * BINS],
            fdl_pos: 0,
            in_block: vec![0.0; FFT_N],
            in_fill: 0,
            out_block: vec![0.0; HEAD],
            work: vec![Complex32::new(0.0, 0.0); FFT_N],
            scratch: vec![Complex32::new(0.0, 0.0); scratch_len],
            fft,
            ifft,
        }
    }

    fn load_ir(&mut self, samples: &[f32]) {
        let n = samples.len().min(MAX_IR_SAMPLES);
        self.ir.clear();
        self.ir.extend_from_slice(&samples[..n]);
        self.effective_len = n;
        self.active_parts = n.saturating_sub(HEAD).div_ceil(HEAD);
        for p in 0..MAX_PARTS {
            self.partition_spectrum(p);
        }
    }

    /// An impulse and its partitions' spectra made elsewhere: copied in,
    /// with no FFT and no allocation, so it can happen while playing.
    fn load_prepared(&mut self, ir: &[f32], parts: &[Complex32]) {
        let n = ir.len().min(MAX_IR_SAMPLES);
        self.ir.clear();
        self.ir.extend_from_slice(&ir[..n]);
        self.parts.copy_from_slice(parts);
        self.effective_len = n;
        self.active_parts = n.saturating_sub(HEAD).div_ceil(HEAD);
    }

    /// Tail partition `p`'s spectrum: its HEAD taps up to the trim,
    /// zero-padded to FFT_N.
    fn partition_spectrum(&mut self, p: usize) {
        let from = HEAD + p * HEAD;
        let end = self.effective_len;
        for (k, w) in self.work.iter_mut().enumerate() {
            let tap = from + k;
            let v = if k < HEAD && tap < end {
                self.ir[tap]
            } else {
                0.0
            };
            *w = Complex32::new(v, 0.0);
        }
        self.fft
            .process_with_scratch(&mut self.work, &mut self.scratch);
        self.parts[p * BINS..(p + 1) * BINS].copy_from_slice(&self.work[..BINS]);
    }

    /// Keep only the first `trim_samples` taps. The partitions holding the
    /// old and the new end are worked out again, so the cut is exact and
    /// a longer trim brings the taps back. One FFT each, no allocation.
    fn set_trim(&mut self, trim_samples: usize) {
        let old_end = self.effective_len;
        self.effective_len = trim_samples.min(self.ir.len());
        self.active_parts = self.effective_len.saturating_sub(HEAD).div_ceil(HEAD);
        for end in [old_end, self.effective_len] {
            if end > HEAD {
                let p = (end - HEAD - 1) / HEAD;
                if p < MAX_PARTS {
                    self.partition_spectrum(p);
                }
            }
        }
    }

    fn reset(&mut self) {
        self.head_ring.fill(0.0);
        self.head_pos = 0;
        self.fdl.fill(Complex32::new(0.0, 0.0));
        self.fdl_pos = 0;
        self.in_block.fill(0.0);
        self.in_fill = 0;
        self.out_block.fill(0.0);
    }

    fn process(&mut self, input: f32) -> f32 {
        // The head, direct: the newest sample against the first taps.
        self.head_ring[self.head_pos] = input;
        let head_len = self.effective_len.min(HEAD);
        let mut head = 0.0_f32;
        for (k, &coef) in self.ir[..head_len].iter().enumerate() {
            head += coef * self.head_ring[(self.head_pos + HEAD - k) % HEAD];
        }
        self.head_pos = (self.head_pos + 1) % HEAD;

        // The tail for this sample was worked out at the end of the
        // previous block.
        let tail = self.out_block[self.in_fill];
        self.in_block[HEAD + self.in_fill] = input;
        self.in_fill += 1;
        if self.in_fill == HEAD {
            self.next_block();
        }
        head + tail
    }

    /// A block of input is complete: keep its spectrum and work out the
    /// tail's output for the next block from every partition.
    fn next_block(&mut self) {
        self.in_fill = 0;
        if self.active_parts == 0 {
            self.out_block.fill(0.0);
            self.in_block.copy_within(HEAD.., 0);
            return;
        }
        for (w, &x) in self.work.iter_mut().zip(self.in_block.iter()) {
            *w = Complex32::new(x, 0.0);
        }
        self.fft
            .process_with_scratch(&mut self.work, &mut self.scratch);
        let at = self.fdl_pos * BINS;
        self.fdl[at..at + BINS].copy_from_slice(&self.work[..BINS]);
        // Y = sum over partitions q of X(newest - q) * H(q).
        self.work.fill(Complex32::new(0.0, 0.0));
        for q in 0..self.active_parts {
            let x = ((self.fdl_pos + MAX_PARTS - q) % MAX_PARTS) * BINS;
            let h = q * BINS;
            for b in 0..BINS {
                self.work[b] += self.fdl[x + b] * self.parts[h + b];
            }
        }
        // A real signal's spectrum mirrors itself.
        for b in 1..HEAD {
            self.work[FFT_N - b] = self.work[b].conj();
        }
        self.ifft
            .process_with_scratch(&mut self.work, &mut self.scratch);
        let scale = 1.0 / FFT_N as f32;
        for (o, w) in self.out_block.iter_mut().zip(self.work[HEAD..].iter()) {
            *o = w.re * scale;
        }
        self.fdl_pos = (self.fdl_pos + 1) % MAX_PARTS;
        self.in_block.copy_within(HEAD.., 0);
    }
}

/// An impulse response with its partitions' spectra worked out, so a
/// reverb can switch to it while playing (see `ConvolutionReverb::prepare_ir`).
pub struct PreparedIr {
    left: Vec<f32>,
    right: Vec<f32>,
    parts_left: Vec<Complex32>,
    parts_right: Vec<Complex32>,
}

/// Stereo convolution reverb with pre-delay, dry/wet mix, IR
/// length trim (low-CPU mode), and optional stereo width + tail EQ.
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

    /// Work out an impulse ahead, off the audio thread: loading it with
    /// `load_ir_stereo` takes hundreds of FFTs and allocates, too much for
    /// a preset change while playing. Leaves the reverb loaded with it.
    pub fn prepare_ir(&mut self, left: &[f32], right: &[f32]) -> PreparedIr {
        self.load_ir_stereo(left, right);
        PreparedIr {
            left: self.left.ir.clone(),
            right: self.right.ir.clone(),
            parts_left: self.left.parts.clone(),
            parts_right: self.right.parts.clone(),
        }
    }

    /// Switch to an impulse made by `prepare_ir`: copies only, safe while
    /// playing.
    pub fn load_prepared(&mut self, ir: &PreparedIr) {
        self.left.load_prepared(&ir.left, &ir.parts_left);
        self.right.load_prepared(&ir.right, &ir.parts_right);
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

    /// The partitioned engine gives what a plain direct convolution gives,
    /// sample for sample, for an impulse many partitions long, from the
    /// first output sample (no added delay); and after a trim, only the
    /// kept taps count.
    #[test]
    fn partitioned_matches_direct_convolution() {
        let mut seed = 7u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let ir: Vec<f32> = (0..5_000)
            .map(|i| rnd() * (-(i as f32) / 1_500.0).exp())
            .collect();
        let input: Vec<f32> = (0..3_000).map(|_| rnd()).collect();
        let direct = |taps: usize, n: usize| -> f32 {
            (0..taps.min(n + 1)).map(|k| ir[k] * input[n - k]).sum()
        };
        let mut ch = ConvolutionChannel::new();
        ch.load_ir(&ir);
        let mut worst = 0.0_f32;
        for (n, &x) in input.iter().enumerate() {
            worst = worst.max((ch.process(x) - direct(ir.len(), n)).abs());
        }
        assert!(worst < 1e-3, "differs from direct convolution by {worst}");

        ch.reset();
        ch.set_trim(1_000);
        let mut worst = 0.0_f32;
        for (n, &x) in input.iter().enumerate() {
            worst = worst.max((ch.process(x) - direct(1_000, n)).abs());
        }
        assert!(worst < 1e-3, "after a trim: {worst}");
    }
}
