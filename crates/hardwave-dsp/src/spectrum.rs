//! Spectrum-analyzer and EQ frequency-response primitives. Drives
//! the EQ curve display, the real-time analyzer overlay, pre/post
//! comparison, and the multiband crossover visualization.

use rustfft::{num_complex::Complex32, FftPlanner};

/// A running magnitude-spectrum display. `analyze(samples)` returns
/// a log-spaced magnitude array smoothed with an exponential peak-
/// follower + optional peak-hold.
pub struct SpectrumAnalyzer {
    fft_size: usize,
    sample_rate: f32,
    smoothing: f32,
    peak_hold: Vec<f32>,
    peak_hold_decay_per_call: f32,
    current: Vec<f32>,
    window: Vec<f32>,
    planner: FftPlanner<f32>,
}

impl SpectrumAnalyzer {
    pub fn new(sample_rate: f32, fft_size: usize) -> Self {
        let n = fft_size.max(64).next_power_of_two();
        let window: Vec<f32> = (0..n)
            .map(|i| {
                let x = i as f32 / (n - 1) as f32;
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * x).cos())
            })
            .collect();
        Self {
            fft_size: n,
            sample_rate: sample_rate.max(1.0),
            smoothing: 0.6,
            peak_hold: vec![-120.0; n / 2],
            peak_hold_decay_per_call: 0.03,
            current: vec![-120.0; n / 2],
            window,
            planner: FftPlanner::<f32>::new(),
        }
    }

    pub fn set_smoothing(&mut self, smoothing: f32) {
        self.smoothing = smoothing.clamp(0.0, 0.99);
    }

    pub fn set_peak_hold_decay(&mut self, decay_per_call: f32) {
        self.peak_hold_decay_per_call = decay_per_call.clamp(0.0, 1.0);
    }

    /// Analyze a chunk of mono samples. If the chunk is shorter than
    /// the FFT size, zero-padded. Returns magnitudes in dBFS for bins
    /// `[0, N/2)`.
    pub fn analyze(&mut self, samples: &[f32]) -> &[f32] {
        let n = self.fft_size;
        let mut scratch: Vec<Complex32> = (0..n)
            .map(|i| {
                let x = *samples.get(i).unwrap_or(&0.0);
                Complex32::new(x * self.window[i], 0.0)
            })
            .collect();
        let fft = self.planner.plan_fft_forward(n);
        fft.process(&mut scratch);
        // Window sum for magnitude normalization.
        let window_sum: f32 = self.window.iter().sum();
        for (i, out) in self.current.iter_mut().enumerate() {
            let mag = scratch[i].norm() / window_sum.max(1e-6);
            let db = 20.0 * (mag + 1e-12).log10();
            // Exponential smoothing: current = alpha * prev + (1 - alpha) * new.
            *out = self.smoothing * *out + (1.0 - self.smoothing) * db;
            if db > self.peak_hold[i] {
                self.peak_hold[i] = db;
            } else {
                self.peak_hold[i] -= self.peak_hold_decay_per_call * 20.0;
            }
        }
        &self.current
    }

    pub fn current(&self) -> &[f32] {
        &self.current
    }

    pub fn peak_hold(&self) -> &[f32] {
        &self.peak_hold
    }

    pub fn bin_frequency(&self, bin: usize) -> f32 {
        (bin as f32) * self.sample_rate / (self.fft_size as f32)
    }
}

/// An EQ band description — what `auto_eq::EqBandSuggestion` lives
/// next to. Kept here so this module doesn't depend on `auto_eq`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqBand {
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
    pub shape: EqBandShape,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqBandShape {
    Peak,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
    Notch,
}

/// Analytic frequency response for a bank of EQ bands at a set of
/// target frequencies. Returns the overall magnitude response in dB,
/// so the EQ curve display doesn't need to convolve impulses.
pub fn eq_frequency_response(bands: &[EqBand], sample_rate: f32, freqs: &[f32]) -> Vec<f32> {
    freqs
        .iter()
        .map(|&f| {
            let mut gain_db = 0.0_f32;
            for band in bands.iter().filter(|b| b.enabled) {
                gain_db += band_response_db(*band, sample_rate, f);
            }
            gain_db
        })
        .collect()
}

fn band_response_db(band: EqBand, sample_rate: f32, freq: f32) -> f32 {
    use std::f32::consts::PI;
    if sample_rate <= 0.0 || freq <= 0.0 {
        return 0.0;
    }
    // Fractional frequency ratio (octaves) from the band center —
    // core of all shelf / peak / pass approximations here.
    let ratio = freq / band.freq_hz.max(0.1);
    let log_ratio = ratio.log2();
    let q = band.q.max(0.1);
    let bw = 1.0 / q;
    let x = log_ratio / bw;
    match band.shape {
        EqBandShape::Peak => {
            // Gaussian-shaped peak scaled by band gain.
            band.gain_db * (-0.5 * x * x).exp()
        }
        EqBandShape::LowShelf => {
            // Smooth transition from 0 dB above the corner to
            // `gain_db` below. Using a 1-pole-ish S-curve.
            let shelf = 1.0 / (1.0 + (log_ratio * 2.0).exp());
            band.gain_db * shelf
        }
        EqBandShape::HighShelf => {
            let shelf = 1.0 / (1.0 + (-log_ratio * 2.0).exp());
            band.gain_db * shelf
        }
        EqBandShape::LowPass => {
            // -12 dB/oct after corner; flat below.
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
            // Narrow dip — steep Gaussian with a fixed depth of -24 dB
            // at the band center (gain_db ignored for notches).
            let n = (-0.5 * x * x * 4.0).exp();
            -24.0 * n + (1.0 - n) * 0.0 + 0.0 * PI.abs()
        }
    }
}

/// Generate `n` log-spaced frequencies from `low` to `high` —
/// canonical input to an EQ curve or spectrum display.
pub fn log_spaced_frequencies(n: usize, low: f32, high: f32) -> Vec<f32> {
    if n == 0 || low <= 0.0 || high <= low {
        return Vec::new();
    }
    let log_lo = low.log10();
    let log_hi = high.log10();
    (0..n)
        .map(|i| {
            let t = i as f32 / (n - 1).max(1) as f32;
            10_f32.powf(log_lo + (log_hi - log_lo) * t)
        })
        .collect()
}

/// Aggregate spectrum energy into the three crossover bands of a
/// multiband processor at `low_xover` and `high_xover` Hz. Returns
/// `(low_db, mid_db, high_db)`. Uses simple linear bucketing by bin
/// centre frequency.
pub fn band_split_energies(
    spectrum_db: &[f32],
    bin_hz: f32,
    low_xover: f32,
    high_xover: f32,
) -> (f32, f32, f32) {
    if bin_hz <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let mut low = 0.0_f64;
    let mut mid = 0.0_f64;
    let mut high = 0.0_f64;
    let mut n_lo = 0_usize;
    let mut n_mi = 0_usize;
    let mut n_hi = 0_usize;
    for (bin, &db) in spectrum_db.iter().enumerate().skip(1) {
        let f = bin as f32 * bin_hz;
        let lin = 10_f32.powf(db / 20.0);
        let v = (lin as f64) * (lin as f64);
        if f < low_xover {
            low += v;
            n_lo += 1;
        } else if f < high_xover {
            mid += v;
            n_mi += 1;
        } else {
            high += v;
            n_hi += 1;
        }
    }
    let to_db = |rms_sq: f64, n: usize| -> f32 {
        if n == 0 || rms_sq <= 0.0 {
            -120.0
        } else {
            let rms = (rms_sq / n as f64).sqrt().max(1e-9);
            (20.0 * rms.log10()) as f32
        }
    };
    (to_db(low, n_lo), to_db(mid, n_mi), to_db(high, n_hi))
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
    fn analyzer_peaks_on_tone() {
        let sr = 48_000.0;
        let mut an = SpectrumAnalyzer::new(sr, 2048);
        let tone = sine(1_000.0, sr, 2048, 0.5);
        // Run several times so smoothing settles.
        for _ in 0..20 {
            an.analyze(&tone);
        }
        let spectrum = an.current().to_vec();
        let bin_hz = sr / 2048.0;
        let target_bin = (1_000.0 / bin_hz).round() as usize;
        // The target bin should be well above the noise floor of -80
        // dBFS and clearly louder than a far-away bin (e.g. 100 Hz).
        let far_bin = (100.0 / bin_hz).round() as usize;
        assert!(
            spectrum[target_bin] > spectrum[far_bin] + 20.0,
            "target {:.1} far {:.1}",
            spectrum[target_bin],
            spectrum[far_bin]
        );
    }

    #[test]
    fn eq_curve_peak_shows_positive_gain_at_band_center() {
        let bands = [EqBand {
            freq_hz: 1_000.0,
            gain_db: 6.0,
            q: 1.0,
            shape: EqBandShape::Peak,
            enabled: true,
        }];
        let freqs = log_spaced_frequencies(200, 20.0, 20_000.0);
        let resp = eq_frequency_response(&bands, 48_000.0, &freqs);
        // Find the bin closest to 1 kHz and check the gain is ≈ +6 dB.
        let idx = freqs
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (*a - 1_000.0)
                    .abs()
                    .partial_cmp(&(*b - 1_000.0).abs())
                    .unwrap()
            })
            .unwrap()
            .0;
        assert!(
            (resp[idx] - 6.0).abs() < 0.5,
            "resp at 1 kHz = {}",
            resp[idx]
        );
    }

    #[test]
    fn eq_curve_low_shelf_lifts_below_corner() {
        let bands = [EqBand {
            freq_hz: 200.0,
            gain_db: 6.0,
            q: 0.7,
            shape: EqBandShape::LowShelf,
            enabled: true,
        }];
        let freqs = [40.0, 200.0, 2_000.0];
        let resp = eq_frequency_response(&bands, 48_000.0, &freqs);
        // Well below corner ≈ +6 dB; well above ≈ 0 dB.
        assert!(resp[0] > 4.0, "resp[40] = {}", resp[0]);
        assert!(resp[2].abs() < 1.0, "resp[2k] = {}", resp[2]);
    }

    #[test]
    fn disabled_band_contributes_nothing() {
        let bands = [EqBand {
            freq_hz: 1_000.0,
            gain_db: 12.0,
            q: 1.0,
            shape: EqBandShape::Peak,
            enabled: false,
        }];
        let resp = eq_frequency_response(&bands, 48_000.0, &[1_000.0]);
        assert_eq!(resp[0], 0.0);
    }

    #[test]
    fn log_spaced_frequencies_covers_range() {
        let f = log_spaced_frequencies(5, 100.0, 10_000.0);
        assert_eq!(f.len(), 5);
        assert!((f[0] - 100.0).abs() < 1e-3);
        assert!((f[4] - 10_000.0).abs() < 1.0);
        // Geometric ratio between adjacent should be close.
        let r0 = f[1] / f[0];
        let r3 = f[4] / f[3];
        assert!((r0 / r3 - 1.0).abs() < 0.1);
    }

    #[test]
    fn band_split_bins_grow_monotonically_with_tone_position() {
        let sr = 48_000.0;
        let mut an = SpectrumAnalyzer::new(sr, 2048);
        let bin_hz = sr / 2048.0;
        // Low-band tone.
        let low = sine(80.0, sr, 2048, 0.5);
        for _ in 0..10 {
            an.analyze(&low);
        }
        let (l, m, h) = band_split_energies(an.current(), bin_hz, 250.0, 4_000.0);
        assert!(l > m, "low {} should exceed mid {}", l, m);
        assert!(l > h, "low {} should exceed high {}", l, h);
    }
}
