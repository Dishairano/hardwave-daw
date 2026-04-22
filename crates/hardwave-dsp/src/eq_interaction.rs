//! EQ interaction primitives — hit-testing for drag-band handles on
//! the curve display, and a linear-phase FIR EQ implementation that
//! avoids the phase-shift of the biquad-cascade EQ.

use crate::spectrum::{EqBand, EqBandShape};
use std::f32::consts::PI;

/// Hit-test result — identifies which EQ band's handle is under the
/// user's pointer when they click on the curve display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandHitTest {
    pub band_index: usize,
    pub distance_squared: f32,
}

/// Return the index of the band whose handle is closest to the
/// pointer, or `None` if no handle is within the tolerance radii.
/// Distance is measured in log-frequency units (octaves) and dB so
/// the hit-testing matches the curve's visual space.
pub fn hit_test_band(
    bands: &[EqBand],
    pointer_freq_hz: f32,
    pointer_gain_db: f32,
    freq_tolerance_octaves: f32,
    gain_tolerance_db: f32,
) -> Option<BandHitTest> {
    if bands.is_empty() || pointer_freq_hz <= 0.0 {
        return None;
    }
    let pointer_octave = pointer_freq_hz.log2();
    let freq_scale = freq_tolerance_octaves.max(1e-3);
    let gain_scale = gain_tolerance_db.max(1e-3);
    let mut best: Option<BandHitTest> = None;
    for (i, band) in bands.iter().enumerate() {
        if !band.enabled {
            continue;
        }
        let dx = (band.freq_hz.max(1e-3).log2() - pointer_octave) / freq_scale;
        let dy = (band.gain_db - pointer_gain_db) / gain_scale;
        let d2 = dx * dx + dy * dy;
        if d2 <= 1.0 {
            let hit = BandHitTest {
                band_index: i,
                distance_squared: d2,
            };
            if best.is_none_or(|b| hit.distance_squared < b.distance_squared) {
                best = Some(hit);
            }
        }
    }
    best
}

/// Design a linear-phase FIR EQ from a set of bands using the
/// frequency-sampling method. Returns an `taps` FIR kernel with
/// symmetric impulse response (so the filter has constant group
/// delay of `(taps - 1) / 2` samples — the defining property of
/// linear-phase mode).
pub fn design_linear_phase_fir(bands: &[EqBand], sample_rate: f32, taps: usize) -> Vec<f32> {
    let taps = taps.max(17);
    let taps = if taps.is_multiple_of(2) {
        taps + 1
    } else {
        taps
    };
    // Desired magnitude response at each bin of the FIR's DFT.
    let half = taps / 2 + 1;
    let mut desired_db = vec![0.0_f32; half];
    for (bin, slot) in desired_db.iter_mut().enumerate() {
        let freq = (bin as f32) * sample_rate / (taps as f32);
        for band in bands.iter().filter(|b| b.enabled) {
            *slot += magnitude_response_db(*band, freq);
        }
    }
    // Convert dB → linear magnitude.
    let desired_mag: Vec<f32> = desired_db.iter().map(|d| 10_f32.powf(d / 20.0)).collect();
    // Build the full spectrum (mirror) with zero phase so the
    // impulse response is symmetric. We put the zero-phase impulse
    // at sample (taps - 1)/2 via circular shift of the inverse FFT
    // result — done here with a direct DFT-based approach for
    // clarity over an FFT.
    let mut h = vec![0.0_f32; taps];
    let center = (taps - 1) as i32 / 2;
    for (n, slot) in h.iter_mut().enumerate() {
        let mut sum = 0.0_f64;
        for (k, mag) in desired_mag.iter().enumerate() {
            let omega = 2.0 * PI * (k as f32) / (taps as f32);
            let nf = n as i32 - center;
            let cos_term = (omega * nf as f32).cos();
            let weight: f64 = if k == 0 || (k == half - 1 && taps.is_multiple_of(2)) {
                1.0
            } else {
                2.0
            };
            sum += (*mag as f64) * (cos_term as f64) * weight;
        }
        *slot = (sum / taps as f64) as f32;
    }
    // Hann window the kernel to reduce ripple.
    for (n, tap) in h.iter_mut().enumerate() {
        let w = 0.5 * (1.0 - (2.0 * PI * (n as f32) / ((taps - 1) as f32)).cos());
        *tap *= w;
    }
    h
}

fn magnitude_response_db(band: EqBand, freq: f32) -> f32 {
    if band.freq_hz <= 0.0 || freq <= 0.0 {
        return 0.0;
    }
    let ratio = freq / band.freq_hz;
    let log_ratio = ratio.log2();
    let q = band.q.max(0.1);
    let bw = 1.0 / q;
    let x = log_ratio / bw;
    match band.shape {
        EqBandShape::Peak => band.gain_db * (-0.5 * x * x).exp(),
        EqBandShape::LowShelf => band.gain_db * (1.0 / (1.0 + (log_ratio * 2.0).exp())),
        EqBandShape::HighShelf => band.gain_db * (1.0 / (1.0 + (-log_ratio * 2.0).exp())),
        EqBandShape::LowPass => {
            if log_ratio <= 0.0 {
                0.0
            } else {
                -12.0 * log_ratio
            }
        }
        EqBandShape::HighPass => {
            if log_ratio >= 0.0 {
                0.0
            } else {
                12.0 * log_ratio
            }
        }
        EqBandShape::Notch => {
            let n = (-0.5 * x * x * 4.0).exp();
            -24.0 * n
        }
    }
}

/// Runtime linear-phase EQ — holds an FIR kernel and applies it via
/// naive convolution over an input buffer. Not optimized; the value
/// is in the *linear-phase* behavior, not in CPU efficiency. Swap
/// for an FFT-based overlap-save implementation when this lives in
/// the real audio graph.
pub struct LinearPhaseEq {
    pub kernel: Vec<f32>,
    pub sample_rate: f32,
}

impl LinearPhaseEq {
    pub fn new(bands: &[EqBand], sample_rate: f32, taps: usize) -> Self {
        Self {
            kernel: design_linear_phase_fir(bands, sample_rate, taps),
            sample_rate,
        }
    }

    pub fn process(&self, input: &[f32]) -> Vec<f32> {
        let k = &self.kernel;
        let mut out = vec![0.0_f32; input.len()];
        for (n, out_slot) in out.iter_mut().enumerate() {
            let mut acc = 0.0_f32;
            for (i, &coeff) in k.iter().enumerate() {
                if n >= i {
                    acc += coeff * input[n - i];
                }
            }
            *out_slot = acc;
        }
        out
    }

    /// Pre-ringing latency in samples — the property the user wanted
    /// when enabling linear-phase mode (constant group delay).
    pub fn latency_samples(&self) -> usize {
        if self.kernel.is_empty() {
            0
        } else {
            (self.kernel.len() - 1) / 2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band(freq: f32, gain_db: f32, shape: EqBandShape) -> EqBand {
        EqBand {
            freq_hz: freq,
            gain_db,
            q: 1.0,
            shape,
            enabled: true,
        }
    }

    #[test]
    fn hit_test_finds_closest_band() {
        let bands = [
            band(100.0, 3.0, EqBandShape::Peak),
            band(1_000.0, 3.0, EqBandShape::Peak),
            band(10_000.0, 3.0, EqBandShape::Peak),
        ];
        let hit = hit_test_band(&bands, 1_050.0, 3.0, 0.5, 3.0).expect("hit");
        assert_eq!(hit.band_index, 1);
    }

    #[test]
    fn hit_test_returns_none_outside_tolerance() {
        let bands = [band(1_000.0, 0.0, EqBandShape::Peak)];
        assert!(hit_test_band(&bands, 8_000.0, 0.0, 0.25, 1.0).is_none());
        assert!(hit_test_band(&bands, 1_000.0, 12.0, 1.0, 3.0).is_none());
    }

    #[test]
    fn hit_test_skips_disabled_bands() {
        let mut bands = [band(1_000.0, 0.0, EqBandShape::Peak)];
        bands[0].enabled = false;
        assert!(hit_test_band(&bands, 1_000.0, 0.0, 1.0, 3.0).is_none());
    }

    #[test]
    fn linear_phase_kernel_is_symmetric() {
        let bands = [band(1_000.0, 6.0, EqBandShape::Peak)];
        let fir = design_linear_phase_fir(&bands, 48_000.0, 65);
        for i in 0..fir.len() / 2 {
            let l = fir[i];
            let r = fir[fir.len() - 1 - i];
            assert!((l - r).abs() < 1e-5, "asymmetric at {}: {} vs {}", i, l, r);
        }
    }

    #[test]
    fn linear_phase_latency_is_half_kernel() {
        let bands = [band(1_000.0, 0.0, EqBandShape::Peak)];
        let eq = LinearPhaseEq::new(&bands, 48_000.0, 65);
        assert_eq!(eq.latency_samples(), 32);
    }

    #[test]
    fn empty_bands_or_all_disabled_produce_kernel_without_shape() {
        let eq_empty = LinearPhaseEq::new(&[], 48_000.0, 33);
        // Kernel exists, just flat magnitude.
        assert_eq!(eq_empty.kernel.len(), 33);
    }

    #[test]
    fn process_passes_through_empty_input() {
        let bands = [band(1_000.0, 3.0, EqBandShape::Peak)];
        let eq = LinearPhaseEq::new(&bands, 48_000.0, 33);
        assert!(eq.process(&[]).is_empty());
    }
}
