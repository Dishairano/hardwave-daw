//! Phase-vocoder time stretching — higher-quality than plain OLA
//! because phases are propagated at each bin's true instantaneous
//! frequency, so tonal content stays coherent across frames.
//!
//! Algorithm:
//! 1. STFT with a Hann window, analysis hop `H_a = N/4`.
//! 2. Per bin, track phase between frames, compute the deviation
//!    from the expected `2π k H_a / N` advance, and derive the true
//!    instantaneous frequency.
//! 3. Propagate a synthesis phase using the true frequency and the
//!    synthesis hop `H_s = H_a * ratio`.
//! 4. IFFT, window, overlap-add at `H_s`.
//!
//! This is the "standard" phase vocoder — no phase-locking across
//! frequency bins (which would be the next quality step).

use rustfft::{num_complex::Complex32, FftPlanner};
use std::f32::consts::PI;

/// Time-stretch `samples` by `ratio` (output length ≈ input × ratio)
/// using a phase vocoder. `n` must be a power of two ≥ 64 and the
/// analysis hop is hard-coded to `n / 4` (75% overlap).
pub fn phase_vocoder_stretch(samples: &[f32], ratio: f32, n: usize) -> Vec<f32> {
    if samples.is_empty() || ratio <= 0.0 || n < 64 || !n.is_power_of_two() {
        return samples.to_vec();
    }
    let h_a = n / 4;
    let h_s = ((h_a as f32) * ratio).round().max(1.0) as usize;
    let window: Vec<f32> = (0..n)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (n - 1) as f32).cos()))
        .collect();

    let num_frames = if samples.len() >= n {
        (samples.len() - n) / h_a + 1
    } else {
        0
    };
    if num_frames == 0 {
        return Vec::new();
    }

    let out_len = (num_frames - 1) * h_s + n;
    let mut out = vec![0.0_f32; out_len];
    let mut weight = vec![0.0_f32; out_len];

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let ifft = planner.plan_fft_inverse(n);

    let mut prev_analysis_phase = vec![0.0_f32; n];
    let mut synthesis_phase = vec![0.0_f32; n];
    let mut scratch: Vec<Complex32> = vec![Complex32::new(0.0, 0.0); n];

    for frame in 0..num_frames {
        let offset = frame * h_a;
        for i in 0..n {
            let x = samples[offset + i] * window[i];
            scratch[i] = Complex32::new(x, 0.0);
        }
        fft.process(&mut scratch);

        for k in 0..n {
            let mag = scratch[k].norm();
            let phase = scratch[k].im.atan2(scratch[k].re);
            let expected = 2.0 * PI * (k as f32) * (h_a as f32) / (n as f32);
            let delta = phase - prev_analysis_phase[k] - expected;
            let wrapped = wrap_pi(delta);
            let true_freq = expected + wrapped;
            let advance = true_freq * (h_s as f32) / (h_a as f32);
            synthesis_phase[k] += advance;
            synthesis_phase[k] = wrap_pi(synthesis_phase[k]);
            prev_analysis_phase[k] = phase;
            let (sin_p, cos_p) = synthesis_phase[k].sin_cos();
            scratch[k] = Complex32::new(mag * cos_p, mag * sin_p);
        }

        ifft.process(&mut scratch);
        let scale = 1.0 / n as f32;
        let out_offset = frame * h_s;
        for i in 0..n {
            if out_offset + i >= out.len() {
                break;
            }
            let sample = scratch[i].re * scale * window[i];
            out[out_offset + i] += sample;
            weight[out_offset + i] += window[i] * window[i];
        }
    }

    for (o, w) in out.iter_mut().zip(weight.iter()) {
        if *w > 1e-6 {
            *o /= *w;
        }
    }
    let target_len = ((samples.len() as f32) * ratio).round() as usize;
    out.truncate(target_len);
    out
}

/// Pitch-shift using the phase vocoder — stretch by `2^(st/12)` then
/// resample back to the original length. Higher-quality than the OLA
/// path in `time_pitch` for tonal material.
pub fn pitch_shift_pv(samples: &[f32], semitones: f32) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let ratio = 2_f32.powf(semitones / 12.0);
    if (ratio - 1.0).abs() < 1e-4 {
        return samples.to_vec();
    }
    let stretched = phase_vocoder_stretch(samples, ratio, 2048);
    crate::time_pitch::resample_to_length(&stretched, samples.len())
}

fn wrap_pi(x: f32) -> f32 {
    let mut y = x;
    while y > PI {
        y -= 2.0 * PI;
    }
    while y < -PI {
        y += 2.0 * PI;
    }
    y
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
    fn stretch_ratio_2_roughly_doubles_length() {
        let sr = 44_100.0;
        let input = sine(440.0, sr, 8192);
        let out = phase_vocoder_stretch(&input, 2.0, 1024);
        let target = input.len() * 2;
        let diff = (out.len() as i32 - target as i32).abs();
        assert!(diff < 2048, "len {} target {}", out.len(), target);
    }

    #[test]
    fn stretch_preserves_pitch_of_sine() {
        let sr = 44_100.0;
        let input = sine(440.0, sr, 16_384);
        let stretched = phase_vocoder_stretch(&input, 1.5, 1024);
        // Count zero crossings per unit time — should match the input
        // rate within ~10%.
        let in_zc_rate = zero_crossings(&input) as f32 / (input.len() as f32 / sr);
        let n = stretched.len().min(8192);
        let mid = stretched.len() / 2;
        let lo = mid.saturating_sub(n / 2);
        let hi = (lo + n).min(stretched.len());
        let out_zc_rate = zero_crossings(&stretched[lo..hi]) as f32 / ((hi - lo) as f32 / sr);
        let rel = (in_zc_rate - out_zc_rate).abs() / in_zc_rate;
        assert!(rel < 0.15, "zc rate drift {:.1}%", rel * 100.0);
    }

    #[test]
    fn pitch_shift_pv_raises_zero_crossing_rate() {
        let sr = 44_100.0;
        let input = sine(220.0, sr, 16_384);
        let shifted = pitch_shift_pv(&input, 12.0);
        let m = input.len() / 2;
        let win = 1024.min(m.min(shifted.len() / 2) - 1);
        let orig_zc = zero_crossings(&input[m - win..m + win]);
        let shifted_zc = zero_crossings(&shifted[m - win..m + win]);
        assert!(shifted_zc as f32 > orig_zc as f32 * 1.5);
    }

    #[test]
    fn pitch_shift_zero_is_identity() {
        let input = sine(440.0, 44_100.0, 4096);
        let out = pitch_shift_pv(&input, 0.0);
        assert_eq!(out.len(), input.len());
        for (a, b) in input.iter().zip(out.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn invalid_params_fall_back_to_input() {
        let input = sine(440.0, 44_100.0, 4096);
        // Non-power-of-two `n` returns the input untouched.
        let out = phase_vocoder_stretch(&input, 1.5, 1000);
        assert_eq!(out.len(), input.len());
    }
}
