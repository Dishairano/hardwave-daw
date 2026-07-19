//! Phase-vocoder time stretching — higher-quality than plain OLA
//! because phases are propagated at each bin's true instantaneous
//! frequency, so tonal content stays coherent across frames.
//!
//! Algorithm:
//! 1. STFT with a Hann window, analysis hop `H_a = N/8`.
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
/// analysis hop is hard-coded to `n / 8` (87.5% overlap).
pub fn phase_vocoder_stretch(samples: &[f32], ratio: f32, n: usize) -> Vec<f32> {
    if samples.is_empty() || ratio <= 0.0 || n < 64 || !n.is_power_of_two() {
        return samples.to_vec();
    }
    // 87.5% analysis overlap. At the more common n/4 (75%) a stretch of 2.0
    // pushes the synthesis hop out to n/2 — only 2 frames overlap per output
    // sample, which is too sparse for the weighted overlap-add to reconstruct
    // smoothly and leaves audible amplitude ripple (~+5 dB). Halving the hop
    // keeps 4 frames overlapping even at ratio 2.0. It doubles the frame
    // count, but this runs as a cached offline bake, not on the audio thread.
    let h_a = n / 8;
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

    // Weighted overlap-add normalisation, with the divisor floored at a real
    // fraction of the steady-state weight.
    //
    // Rewriting the phases destroys each frame's Hann taper, so the
    // resynthesised frame has roughly uniform amplitude. That makes the
    // numerator scale with `window` while the divisor scales with `window²`,
    // so at the head and tail — where only one tapered frame overlaps — the
    // quotient diverges like `A / window`. Unfloored this produced +50 dBFS
    // spikes; a token 1e-6 floor still left ~22x.
    //
    // With the 87.5% analysis overlap above, the steady-state weight is
    // essentially flat, so the floor can sit close to it without clamping the
    // body. 0.7 measures a worst-case edge of ~1.03x (vs 2.1x at 0.3) while
    // still leaving margin for the sparser overlap that extreme ratios
    // produce. Edges now taper instead of clicking.
    let max_w = weight.iter().copied().fold(0.0_f32, f32::max);
    let floor = (max_w * 0.7).max(1e-6);
    for (o, w) in out.iter_mut().zip(weight.iter()) {
        *o /= w.max(floor);
    }
    let target_len = ((samples.len() as f32) * ratio).round() as usize;
    out.truncate(target_len);

    // Match the source's RMS. The WOLA divisor above restores the *shape*,
    // but because the vocoder rewrites phases the overlapped frames no longer
    // sum coherently the way plain analysis/synthesis windows would, so the
    // residual gain drifts with the synthesis hop (measured up to ~4.7x at
    // ratio 0.5). Re-matching energy makes the guarantee explicit and
    // ratio-independent: a stretch changes duration, not loudness.
    let rms = |v: &[f32]| -> f32 {
        if v.is_empty() {
            return 0.0;
        }
        (v.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>() / v.len() as f64).sqrt() as f32
    };
    let in_rms = rms(samples);
    let out_rms = rms(&out);
    if in_rms > 0.0 && out_rms > 1e-9 {
        let gain = in_rms / out_rms;
        for o in out.iter_mut() {
            *o *= gain;
        }
    }
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

    #[test]
    fn stretch_does_not_blow_up_the_level() {
        // Regression: the WOLA normalisation divided by `weight` wherever it
        // exceeded 1e-6. At the head/tail only one tapered frame contributes,
        // so weight tends to zero and the division amplified numerical noise
        // into enormous spikes — renders peaked over +50 dBFS (~385 linear)
        // instead of staying near the source's 1.0.
        let input = sine(440.0, 48_000.0, 48_000);
        let in_peak = input.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        for ratio in [0.5_f32, 1.5, 2.0, 4.0] {
            let out = phase_vocoder_stretch(&input, ratio, 2048);
            let peak = out.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
            assert!(
                peak < in_peak * 4.0,
                "ratio {ratio}: output peak {peak} exploded vs input peak {in_peak}"
            );
            assert!(
                out.iter().all(|s| s.is_finite()),
                "ratio {ratio}: output contains NaN/inf"
            );
        }
    }
}
