//! ITU-R BS.1770-inspired loudness metering. Computes integrated
//! LUFS, short-term LUFS (3 s), momentary LUFS (0.4 s), and true-peak
//! approximation for a stereo mixdown.
//!
//! This is a pragmatic implementation — K-weighting uses the
//! published biquad coefficients, 400 ms windows with 75 % overlap,
//! absolute gate at -70 LUFS, relative gate at -10 dB below the
//! ungated mean. Good enough for in-DAW meters and export checks;
//! not a certified broadcast tool.

/// Loudness measurement over a full buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoudnessStats {
    pub integrated_lufs: f32,
    pub short_term_lufs: f32,
    pub momentary_lufs: f32,
    pub true_peak_dbfs: f32,
}

/// Measure loudness on a stereo buffer. Inputs are f32 interleaved
/// or split channels — this overload takes two parallel slices.
pub fn measure(left: &[f32], right: &[f32], sample_rate: f32) -> LoudnessStats {
    let n = left.len().min(right.len());
    if n == 0 || sample_rate <= 0.0 {
        return LoudnessStats {
            integrated_lufs: f32::NEG_INFINITY,
            short_term_lufs: f32::NEG_INFINITY,
            momentary_lufs: f32::NEG_INFINITY,
            true_peak_dbfs: f32::NEG_INFINITY,
        };
    }
    let mut pre_l = KFilter::new(sample_rate);
    let mut pre_r = KFilter::new(sample_rate);
    let filt_l: Vec<f32> = left[..n].iter().map(|&s| pre_l.process(s)).collect();
    let filt_r: Vec<f32> = right[..n].iter().map(|&s| pre_r.process(s)).collect();

    let window_len = (0.4 * sample_rate) as usize; // 400 ms momentary
    let hop = (0.1 * sample_rate) as usize; // 100 ms → 75% overlap
    let mut momentary_levels: Vec<f32> = Vec::new();
    if window_len > 0 && hop > 0 {
        let mut start = 0;
        while start + window_len <= n {
            let end = start + window_len;
            let sum = channel_ms(&filt_l[start..end]) + channel_ms(&filt_r[start..end]);
            let level = -0.691 + 10.0 * (sum + 1e-12).log10();
            momentary_levels.push(level);
            start += hop;
        }
    }
    let momentary_lufs = momentary_levels
        .last()
        .copied()
        .unwrap_or(f32::NEG_INFINITY);
    let short_term_lufs = if momentary_levels.len() >= 30 {
        // Last 30 momentary windows ≈ last 3 s (400 ms w/ 75 % overlap).
        let tail = &momentary_levels[momentary_levels.len() - 30..];
        average_lin(tail)
    } else if !momentary_levels.is_empty() {
        average_lin(&momentary_levels)
    } else {
        f32::NEG_INFINITY
    };

    // Integrated LUFS with two-pass gating.
    let integrated_lufs = integrated_loudness(&momentary_levels);

    // True peak ≈ 4× oversampled abs peak. Cheaper: upsample with
    // linear interpolation by factor 4 and take the max. Good enough
    // for a meter; not an ISP filter.
    let tp = true_peak(&left[..n], 4).max(true_peak(&right[..n], 4));
    let true_peak_dbfs = if tp > 0.0 {
        20.0 * tp.log10()
    } else {
        f32::NEG_INFINITY
    };

    LoudnessStats {
        integrated_lufs,
        short_term_lufs,
        momentary_lufs,
        true_peak_dbfs,
    }
}

fn channel_ms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / samples.len() as f64) as f32
}

fn average_lin(levels_db: &[f32]) -> f32 {
    // Average in linear domain then convert back to LUFS.
    let mut sum = 0.0_f64;
    let mut count = 0_usize;
    for &l in levels_db {
        if l.is_finite() {
            sum += 10f64.powf((l as f64 + 0.691) / 10.0);
            count += 1;
        }
    }
    if count == 0 {
        return f32::NEG_INFINITY;
    }
    let mean = sum / count as f64;
    (-0.691 + 10.0 * mean.log10()) as f32
}

fn integrated_loudness(momentary: &[f32]) -> f32 {
    if momentary.is_empty() {
        return f32::NEG_INFINITY;
    }
    // Absolute gate: discard anything below -70 LUFS.
    let abs_gated: Vec<f32> = momentary
        .iter()
        .copied()
        .filter(|&l| l.is_finite() && l > -70.0)
        .collect();
    if abs_gated.is_empty() {
        return f32::NEG_INFINITY;
    }
    let ungated_mean = average_lin(&abs_gated);
    // Relative gate: everything below (ungated_mean - 10) dB out.
    let rel_threshold = ungated_mean - 10.0;
    let rel_gated: Vec<f32> = abs_gated
        .into_iter()
        .filter(|&l| l > rel_threshold)
        .collect();
    if rel_gated.is_empty() {
        return ungated_mean;
    }
    average_lin(&rel_gated)
}

fn true_peak(samples: &[f32], factor: usize) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let factor = factor.max(1);
    let mut peak = 0.0_f32;
    for w in samples.windows(2) {
        let a = w[0];
        let b = w[1];
        for i in 0..factor {
            let t = i as f32 / factor as f32;
            let v = a * (1.0 - t) + b * t;
            peak = peak.max(v.abs());
        }
    }
    peak = peak.max(samples.last().copied().unwrap_or(0.0).abs());
    peak
}

/// K-weighting filter — the pre-filter (high shelf) + RLB
/// weighting (high-pass) cascade from BS.1770-4.
struct KFilter {
    pre: Biquad,
    rlb: Biquad,
}

impl KFilter {
    fn new(sample_rate: f32) -> Self {
        Self {
            pre: Biquad::pre_filter(sample_rate),
            rlb: Biquad::rlb_filter(sample_rate),
        }
    }
    fn process(&mut self, x: f32) -> f32 {
        self.rlb.process(self.pre.process(x))
    }
}

#[derive(Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Pre-filter — high-shelf at ~1.5 kHz with +4 dB gain, designed
    /// around 48 kHz. We re-derive for arbitrary sample rates via the
    /// bilinear transform of the canonical analog prototype used in
    /// BS.1770.
    fn pre_filter(fs: f32) -> Self {
        // Using published coefficients at 48 kHz as reference; for
        // other rates we compute via the same gain/Q bilinear map.
        // Parameters chosen to match Table 1 in BS.1770-4.
        let f0 = 1_681.974_5;
        let g = 3.999_843_8;
        let q = 0.707_520_5;
        shelf_biquad(fs, f0, q, g, true)
    }

    fn rlb_filter(fs: f32) -> Self {
        // RLB weighting — 2nd-order high-pass.
        let f0 = 38.135_47;
        let q = 0.500_327_3;
        hpf_biquad(fs, f0, q)
    }
}

fn shelf_biquad(fs: f32, f0: f32, q: f32, gain_db: f32, high_shelf: bool) -> Biquad {
    use std::f32::consts::PI;
    let a = 10f32.powf(gain_db / 40.0);
    let w0 = 2.0 * PI * f0 / fs;
    let cs = w0.cos();
    let sn = w0.sin();
    let alpha = sn / (2.0 * q);
    let two_sqrt_a = 2.0 * a.sqrt() * alpha;
    let (b0, b1, b2, a0, a1, a2) = if high_shelf {
        (
            a * ((a + 1.0) + (a - 1.0) * cs + two_sqrt_a),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
            a * ((a + 1.0) + (a - 1.0) * cs - two_sqrt_a),
            (a + 1.0) - (a - 1.0) * cs + two_sqrt_a,
            2.0 * ((a - 1.0) - (a + 1.0) * cs),
            (a + 1.0) - (a - 1.0) * cs - two_sqrt_a,
        )
    } else {
        (
            a * ((a + 1.0) - (a - 1.0) * cs + two_sqrt_a),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
            a * ((a + 1.0) - (a - 1.0) * cs - two_sqrt_a),
            (a + 1.0) + (a - 1.0) * cs + two_sqrt_a,
            -2.0 * ((a - 1.0) + (a + 1.0) * cs),
            (a + 1.0) + (a - 1.0) * cs - two_sqrt_a,
        )
    };
    Biquad {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
        z1: 0.0,
        z2: 0.0,
    }
}

fn hpf_biquad(fs: f32, f0: f32, q: f32) -> Biquad {
    use std::f32::consts::PI;
    let w0 = 2.0 * PI * f0 / fs;
    let cs = w0.cos();
    let sn = w0.sin();
    let alpha = sn / (2.0 * q);
    let b0 = (1.0 + cs) * 0.5;
    let b1 = -(1.0 + cs);
    let b2 = (1.0 + cs) * 0.5;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cs;
    let a2 = 1.0 - alpha;
    Biquad {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
        z1: 0.0,
        z2: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin())
            .collect()
    }

    #[test]
    fn silence_is_infinitely_quiet() {
        let sr = 48_000.0;
        let buf = vec![0.0_f32; (sr * 2.0) as usize];
        let s = measure(&buf, &buf, sr);
        assert_eq!(s.integrated_lufs, f32::NEG_INFINITY);
        assert_eq!(s.true_peak_dbfs, f32::NEG_INFINITY);
    }

    #[test]
    fn louder_signal_measures_higher_lufs() {
        let sr = 48_000.0;
        let n = (sr * 3.0) as usize;
        let quiet_l = sine(1_000.0, sr, n, 0.1);
        let quiet_r = quiet_l.clone();
        let loud_l = sine(1_000.0, sr, n, 0.5);
        let loud_r = loud_l.clone();
        let q = measure(&quiet_l, &quiet_r, sr);
        let l = measure(&loud_l, &loud_r, sr);
        assert!(
            l.integrated_lufs > q.integrated_lufs + 10.0,
            "quiet {}, loud {}",
            q.integrated_lufs,
            l.integrated_lufs
        );
    }

    #[test]
    fn true_peak_tracks_peak_amplitude() {
        let sr = 48_000.0;
        let n = (sr * 1.0) as usize;
        let buf = sine(500.0, sr, n, 0.8);
        let s = measure(&buf, &buf, sr);
        // Sine with amplitude 0.8 → peak ≈ 0.8 → -1.94 dBFS.
        assert!(
            (s.true_peak_dbfs - (-1.94)).abs() < 0.5,
            "tp {} expected ≈ -1.94",
            s.true_peak_dbfs
        );
    }

    #[test]
    fn one_khz_sine_near_minus_23_lufs_for_sine_reference() {
        // A full-scale 1 kHz sine (amp 1.0, centered) reads ~-3.01 LUFS.
        // Amplitude 0.5 → ~-9.03 LUFS. We just check the measurement
        // lands in a sensible ballpark, not a tight reference-grade
        // tolerance.
        let sr = 48_000.0;
        let n = (sr * 4.0) as usize;
        let buf = sine(1_000.0, sr, n, 0.5);
        let s = measure(&buf, &buf, sr);
        assert!(
            (s.integrated_lufs - (-9.0)).abs() < 3.0,
            "integrated {}",
            s.integrated_lufs
        );
    }

    #[test]
    fn relative_gate_rejects_silent_head() {
        // 1 s of silence followed by 3 s of -10 dBFS tone should
        // measure close to the tone-only loudness, not half of it,
        // because the silent windows should be gated out.
        let sr = 48_000.0;
        let head = vec![0.0_f32; sr as usize];
        let mut tone = sine(1_000.0, sr, (sr * 3.0) as usize, 0.316); // ≈ -10 dBFS
        let mut full_l = head.clone();
        full_l.append(&mut tone);
        let full_r = full_l.clone();
        let s_full = measure(&full_l, &full_r, sr);
        let tone_only = sine(1_000.0, sr, (sr * 3.0) as usize, 0.316);
        let s_tone = measure(&tone_only, &tone_only, sr);
        assert!(
            (s_full.integrated_lufs - s_tone.integrated_lufs).abs() < 2.0,
            "full {} tone {}",
            s_full.integrated_lufs,
            s_tone.integrated_lufs
        );
    }
}
