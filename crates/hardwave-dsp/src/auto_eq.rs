//! Smart auto-EQ helpers. Two deterministic primitives:
//!
//! * `detect_resonances` — windowed FFT peak-picker that returns a
//!   list of suggested narrow-Q cuts where the spectrum spikes above
//!   the smoothed local average. Useful for "remove resonances" on
//!   a problematic source (muddy vocal, ringing snare).
//! * `match_reference_tilt` — compares the target and reference band
//!   energies and returns a small set of shelving-band suggestions
//!   that tilt the target toward the reference's tonal balance.
//!
//! Both return `EqBandSuggestion`s that downstream code can feed
//! straight into `parametric_eq::Biquad` coefficient builders. No ML.

use rustfft::{num_complex::Complex32, FftPlanner};

/// A single suggested EQ move — frequency, gain in dB, Q/bandwidth,
/// and band shape. Designed to map 1:1 onto `parametric_eq` bands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqBandSuggestion {
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
    pub shape: BandShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandShape {
    Peak,
    LowShelf,
    HighShelf,
}

/// Find up to `max_cuts` narrow resonances in the buffer and return
/// cut suggestions (negative gain, moderate Q).
pub fn detect_resonances(
    samples: &[f32],
    sample_rate: f32,
    max_cuts: usize,
) -> Vec<EqBandSuggestion> {
    if max_cuts == 0 {
        return Vec::new();
    }
    let Some(spectrum) = magnitude_spectrum(samples, sample_rate) else {
        return Vec::new();
    };
    let smoothed = moving_average(&spectrum, 21);
    let mut candidates: Vec<(usize, f32)> = Vec::new();
    // A resonance is a bin that's a local maximum AND sticks up at
    // least 6 dB above the smoothed local average.
    for i in 2..spectrum.len() - 2 {
        let v = spectrum[i];
        if v <= spectrum[i - 1] || v <= spectrum[i + 1] {
            continue;
        }
        let baseline = smoothed[i].max(1e-9);
        let ratio_db = 20.0 * (v / baseline).log10();
        if ratio_db >= 6.0 {
            candidates.push((i, ratio_db));
        }
    }
    // Sort by prominence descending, then keep only non-overlapping
    // peaks (spaced at least half an octave apart).
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let n = spectrum.len();
    let bin_hz = sample_rate / (2.0 * n as f32);
    let mut chosen: Vec<(usize, f32)> = Vec::new();
    for (bin, prominence) in candidates {
        let f = bin as f32 * bin_hz;
        let too_close = chosen.iter().any(|(c, _)| {
            let fc = *c as f32 * bin_hz;
            let ratio = if f > fc {
                f / fc.max(1.0)
            } else {
                fc / f.max(1.0)
            };
            ratio < (2.0_f32).sqrt() // half-octave
        });
        if !too_close {
            chosen.push((bin, prominence));
            if chosen.len() >= max_cuts {
                break;
            }
        }
    }
    chosen
        .into_iter()
        .map(|(bin, prominence)| {
            let freq = (bin as f32 * bin_hz).clamp(30.0, sample_rate / 2.0 - 1.0);
            // Scale cut depth to how prominent the resonance is —
            // clamp to [-9, -3] dB so we never over-cut.
            let gain_db = (-prominence * 0.5).clamp(-9.0, -3.0);
            EqBandSuggestion {
                freq_hz: freq,
                gain_db,
                q: 6.0,
                shape: BandShape::Peak,
            }
        })
        .collect()
}

/// Compare target vs. reference 3-band energy split and return a
/// small set of shelf/peak suggestions that tilt the target toward
/// the reference.
pub fn match_reference_tilt(
    target: &[f32],
    reference: &[f32],
    sample_rate: f32,
) -> Vec<EqBandSuggestion> {
    let Some(t_spec) = magnitude_spectrum(target, sample_rate) else {
        return Vec::new();
    };
    let Some(r_spec) = magnitude_spectrum(reference, sample_rate) else {
        return Vec::new();
    };
    let n = t_spec.len().min(r_spec.len());
    let bin_hz = sample_rate / (2.0 * t_spec.len() as f32);
    let mut t_low = 0.0;
    let mut t_mid = 0.0;
    let mut t_high = 0.0;
    let mut r_low = 0.0;
    let mut r_mid = 0.0;
    let mut r_high = 0.0;
    for i in 1..n {
        let f = i as f32 * bin_hz;
        let tv = t_spec[i] * t_spec[i];
        let rv = r_spec[i] * r_spec[i];
        if f < 250.0 {
            t_low += tv;
            r_low += rv;
        } else if f < 4000.0 {
            t_mid += tv;
            r_mid += rv;
        } else {
            t_high += tv;
            r_high += rv;
        }
    }
    let tilt = |target_e: f32, ref_e: f32| -> f32 {
        if target_e <= 0.0 || ref_e <= 0.0 {
            0.0
        } else {
            // Positive = reference louder → lift target. Clamp to ±6 dB.
            (10.0 * (ref_e / target_e).log10()).clamp(-6.0, 6.0)
        }
    };
    let low_db = tilt(t_low, r_low);
    let mid_db = tilt(t_mid, r_mid);
    let high_db = tilt(t_high, r_high);
    let mut out = Vec::new();
    if low_db.abs() > 0.5 {
        out.push(EqBandSuggestion {
            freq_hz: 120.0,
            gain_db: low_db,
            q: 0.7,
            shape: BandShape::LowShelf,
        });
    }
    if mid_db.abs() > 0.5 {
        out.push(EqBandSuggestion {
            freq_hz: 1_000.0,
            gain_db: mid_db,
            q: 0.9,
            shape: BandShape::Peak,
        });
    }
    if high_db.abs() > 0.5 {
        out.push(EqBandSuggestion {
            freq_hz: 8_000.0,
            gain_db: high_db,
            q: 0.7,
            shape: BandShape::HighShelf,
        });
    }
    out
}

fn magnitude_spectrum(samples: &[f32], sample_rate: f32) -> Option<Vec<f32>> {
    if samples.is_empty() || sample_rate <= 0.0 {
        return None;
    }
    let n = samples.len().min(16_384).next_power_of_two().max(1024);
    let mut buf: Vec<Complex32> = (0..n)
        .map(|i| {
            let x = *samples.get(i).unwrap_or(&0.0);
            let w = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos());
            Complex32::new(x * w, 0.0)
        })
        .collect();
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    fft.process(&mut buf);
    let spectrum: Vec<f32> = buf.iter().take(n / 2).map(|c| c.norm()).collect();
    let total: f32 = spectrum.iter().sum();
    if total < 1e-5 {
        return None;
    }
    Some(spectrum)
}

fn moving_average(input: &[f32], width: usize) -> Vec<f32> {
    if width <= 1 || input.is_empty() {
        return input.to_vec();
    }
    let half = width / 2;
    let n = input.len();
    let mut out = vec![0.0_f32; n];
    for (i, slot) in out.iter_mut().enumerate() {
        let lo = i.saturating_sub(half);
        let hi = (i + half).min(n - 1);
        let slice = &input[lo..=hi];
        *slot = slice.iter().sum::<f32>() / slice.len() as f32;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin())
            .collect()
    }

    fn mix(parts: &[Vec<f32>]) -> Vec<f32> {
        let n = parts.iter().map(|p| p.len()).min().unwrap_or(0);
        (0..n).map(|i| parts.iter().map(|p| p[i]).sum()).collect()
    }

    #[test]
    fn resonance_at_1khz_is_detected() {
        let sr = 44_100.0;
        // Broadband noise + a strong 1 kHz peak. The peak should pop
        // above the noise floor.
        let mut rng: u32 = 0x1234;
        let noise: Vec<f32> = (0..16_384)
            .map(|_| {
                rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((rng >> 8) as f32 / 8_388_608.0 - 1.0) * 0.05
            })
            .collect();
        let tone = sine(1_000.0, sr, 16_384, 1.0);
        let mixed = mix(&[noise, tone]);
        let cuts = detect_resonances(&mixed, sr, 3);
        assert!(!cuts.is_empty());
        // The top suggestion should be within a semitone (≈6%) of 1 kHz.
        let top = cuts[0];
        assert!(
            (top.freq_hz / 1_000.0).log2().abs() < 1.0 / 12.0,
            "freq {:.1} Hz",
            top.freq_hz
        );
        assert!(top.gain_db < 0.0, "cut suggestion must be negative gain");
        assert_eq!(top.shape, BandShape::Peak);
    }

    #[test]
    fn resonance_cut_depth_is_clamped() {
        let sr = 44_100.0;
        let tone = sine(500.0, sr, 16_384, 1.0);
        let cuts = detect_resonances(&tone, sr, 2);
        for c in cuts {
            assert!(c.gain_db >= -9.0 && c.gain_db <= -3.0, "gain {}", c.gain_db);
        }
    }

    #[test]
    fn max_cuts_is_respected() {
        let sr = 44_100.0;
        let mixed = mix(&[
            sine(200.0, sr, 16_384, 0.5),
            sine(500.0, sr, 16_384, 0.5),
            sine(1_000.0, sr, 16_384, 0.5),
            sine(2_000.0, sr, 16_384, 0.5),
            sine(4_000.0, sr, 16_384, 0.5),
        ]);
        let cuts = detect_resonances(&mixed, sr, 3);
        assert!(cuts.len() <= 3);
    }

    #[test]
    fn reference_tilt_lifts_quiet_low_end() {
        let sr = 44_100.0;
        // Target: 10 kHz sine only. Reference: 80 Hz sine only.
        // Expect a low-shelf lift and/or high-shelf cut.
        let target = sine(10_000.0, sr, 16_384, 0.5);
        let reference = sine(80.0, sr, 16_384, 0.5);
        let bands = match_reference_tilt(&target, &reference, sr);
        assert!(!bands.is_empty());
        let has_low_lift = bands
            .iter()
            .any(|b| b.shape == BandShape::LowShelf && b.gain_db > 0.0);
        let has_high_cut = bands
            .iter()
            .any(|b| b.shape == BandShape::HighShelf && b.gain_db < 0.0);
        assert!(
            has_low_lift || has_high_cut,
            "expected low lift or high cut, got {:?}",
            bands
        );
    }

    #[test]
    fn silence_returns_no_cuts() {
        let sr = 44_100.0;
        let silence = vec![0.0_f32; 4096];
        assert!(detect_resonances(&silence, sr, 4).is_empty());
        assert!(match_reference_tilt(&silence, &silence, sr).is_empty());
    }
}
