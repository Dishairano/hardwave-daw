//! Time-stretching and pitch-shifting primitives.
//!
//! * `time_stretch_ola(samples, ratio, window, hop)` — classic
//!   overlap-add time stretch. Preserves pitch, changes duration.
//!   Good enough for loops and percussion; smears transients on
//!   extreme ratios (phase vocoder is a follow-up).
//! * `pitch_shift(samples, semitones)` — resample after an OLA
//!   stretch so the duration stays intact but pitch moves.
//! * `resample_linear(samples, ratio)` — linear-interpolation
//!   resampler exposed for callers who only want a sample-rate
//!   change.

use std::f32::consts::PI;

/// Time-stretch `samples` by `ratio` (output length ≈ input length ×
/// `ratio`) using a Hann-windowed OLA. `window` must be at least 4 and
/// `hop` must be < `window`.
pub fn time_stretch_ola(samples: &[f32], ratio: f32, window: usize, hop: usize) -> Vec<f32> {
    if samples.is_empty() || ratio <= 0.0 || window < 4 || hop == 0 || hop >= window {
        return samples.to_vec();
    }
    let input_hop = ((hop as f32) / ratio).round().max(1.0) as usize;
    let out_len = ((samples.len() as f32) * ratio).round() as usize + window;
    let mut out = vec![0.0_f32; out_len];
    let mut weight = vec![0.0_f32; out_len];
    let win: Vec<f32> = (0..window)
        .map(|i| {
            let x = (i as f32) / ((window - 1) as f32);
            0.5 * (1.0 - (2.0 * PI * x).cos())
        })
        .collect();

    let mut input_pos = 0_usize;
    let mut output_pos = 0_usize;
    while input_pos + window <= samples.len() && output_pos + window <= out.len() {
        for k in 0..window {
            let s = samples[input_pos + k] * win[k];
            out[output_pos + k] += s;
            weight[output_pos + k] += win[k];
        }
        input_pos += input_hop;
        output_pos += hop;
    }

    // Normalize by the window weight so plateaus don't double in
    // amplitude. Guard against zero weight at the tails.
    for (o, w) in out.iter_mut().zip(weight.iter()) {
        if *w > 1e-6 {
            *o /= *w;
        }
    }

    // Trim to the expected output length.
    let target_len = ((samples.len() as f32) * ratio).round() as usize;
    out.truncate(target_len);
    out
}

/// Pitch-shift `samples` by `semitones` without changing the
/// duration. Internally: OLA time-stretch by `1 / pitch_ratio`, then
/// linear-resample back to the original length with the reciprocal
/// rate (net pitch shift; length preserved).
pub fn pitch_shift(samples: &[f32], semitones: f32) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let pitch_ratio = 2_f32.powf(semitones / 12.0);
    if (pitch_ratio - 1.0).abs() < 1e-4 {
        return samples.to_vec();
    }
    let window = 1024;
    let hop = 256;
    let stretched = time_stretch_ola(samples, pitch_ratio, window, hop);
    // Resample back: output length = input length.
    let target_len = samples.len();
    resample_to_length(&stretched, target_len)
}

/// Linear-interpolation resampler. `ratio > 1.0` → fewer output
/// samples (speed up); `ratio < 1.0` → more output samples (slow
/// down). Returns an empty buffer on invalid input.
pub fn resample_linear(samples: &[f32], ratio: f32) -> Vec<f32> {
    if samples.is_empty() || ratio <= 0.0 || !ratio.is_finite() {
        return Vec::new();
    }
    let out_len = ((samples.len() as f32) / ratio).round() as usize;
    resample_to_length(samples, out_len)
}

/// Resample `samples` so the output has exactly `target_len` samples.
/// Uses linear interpolation between adjacent source samples.
pub fn resample_to_length(samples: &[f32], target_len: usize) -> Vec<f32> {
    if samples.is_empty() || target_len == 0 {
        return Vec::new();
    }
    if samples.len() == 1 {
        return vec![samples[0]; target_len];
    }
    let src_len = samples.len();
    let mut out = Vec::with_capacity(target_len);
    for i in 0..target_len {
        let t = (i as f32) * (src_len as f32 - 1.0) / (target_len as f32 - 1.0).max(1.0);
        let t0 = t.floor() as usize;
        let t1 = (t0 + 1).min(src_len - 1);
        let frac = t - t0 as f32;
        let v = samples[t0] * (1.0 - frac) + samples[t1] * frac;
        out.push(v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sr).sin())
            .collect()
    }

    fn zero_crossings(samples: &[f32]) -> usize {
        let mut count = 0;
        for w in samples.windows(2) {
            if (w[0] < 0.0 && w[1] >= 0.0) || (w[0] >= 0.0 && w[1] < 0.0) {
                count += 1;
            }
        }
        count
    }

    #[test]
    fn time_stretch_doubles_length_at_ratio_2() {
        let sr = 44_100.0;
        let input = sine(440.0, sr, 4096);
        let out = time_stretch_ola(&input, 2.0, 1024, 256);
        let target = input.len() * 2;
        let diff = (out.len() as i32 - target as i32).abs();
        assert!(diff < 2048, "len {}, target {}", out.len(), target);
    }

    #[test]
    fn time_stretch_halves_length_at_ratio_half() {
        let sr = 44_100.0;
        let input = sine(440.0, sr, 8192);
        let out = time_stretch_ola(&input, 0.5, 1024, 256);
        let target = input.len() / 2;
        let diff = (out.len() as i32 - target as i32).abs();
        assert!(diff < 1024);
    }

    #[test]
    fn pitch_shift_up_increases_zero_crossings() {
        let sr = 44_100.0;
        let input = sine(220.0, sr, 8192);
        let shifted = pitch_shift(&input, 12.0); // up one octave
                                                 // Compare zero-crossings in the stable middle region.
        let m = input.len() / 2;
        let orig_zc = zero_crossings(&input[m - 1024..m + 1024]);
        let shifted_zc = zero_crossings(&shifted[m - 1024..m + 1024]);
        assert!(
            shifted_zc as f32 > orig_zc as f32 * 1.5,
            "orig {} shifted {}",
            orig_zc,
            shifted_zc
        );
    }

    #[test]
    fn pitch_shift_preserves_length() {
        let input = sine(440.0, 44_100.0, 6000);
        let out = pitch_shift(&input, 7.0);
        assert_eq!(out.len(), input.len());
    }

    #[test]
    fn pitch_shift_zero_is_identity_within_tolerance() {
        let input = sine(440.0, 44_100.0, 4096);
        let out = pitch_shift(&input, 0.0);
        assert_eq!(out.len(), input.len());
        // Exactly equal because of the early-return fast path.
        for (a, b) in input.iter().zip(out.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn resample_doubles_length_at_half_ratio() {
        let input = vec![0.0, 1.0, 0.0, -1.0];
        let out = resample_linear(&input, 0.5);
        // Output should be roughly 8 samples.
        assert!(out.len() >= 7 && out.len() <= 9);
    }

    #[test]
    fn resample_to_length_midpoint_interpolates() {
        let input = vec![0.0_f32, 2.0];
        let out = resample_to_length(&input, 3);
        assert_eq!(out.len(), 3);
        assert!((out[1] - 1.0).abs() < 1e-4, "midpoint {}", out[1]);
    }
}
