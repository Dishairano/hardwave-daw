//! Biquad filter — 2-pole, 2-zero IIR with stereo-independent state.
//! Coefficient generation is RBJ Audio EQ Cookbook for the standard
//! response types. The filter is allocation-free and sample-accurate;
//! it's the building block for every filter-based plugin (EQ bands,
//! sidechain HPF/LPF on comp/gate, tone knob on distortion, damping
//! on reverb, feedback filters on delay, etc.).

use std::f32::consts::PI;

/// All RBJ-cookbook filter shapes a biquad can embody.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BiquadKind {
    LowPass,
    HighPass,
    BandPass,
    /// All-through except a narrow dip at cutoff — notch filter.
    Notch,
    /// Parametric peak (bell) — gain controls dB boost/cut at cutoff.
    Peak,
    /// Low shelf — gain controls dB boost/cut below cutoff.
    LowShelf,
    /// High shelf — gain controls dB boost/cut above cutoff.
    HighShelf,
}

/// 2-pole, 2-zero IIR biquad with stereo state. Coefficients are
/// shared across channels; delay lines are per-channel so we can
/// process true stereo with one coefficient set.
#[derive(Default, Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1_l: f32,
    x2_l: f32,
    y1_l: f32,
    y2_l: f32,
    x1_r: f32,
    x2_r: f32,
    y1_r: f32,
    y2_r: f32,
}

impl Biquad {
    /// Zero out the delay lines without touching the coefficients.
    /// Call after setting coefficients if you want a clean transient.
    pub fn reset(&mut self) {
        self.x1_l = 0.0;
        self.x2_l = 0.0;
        self.y1_l = 0.0;
        self.y2_l = 0.0;
        self.x1_r = 0.0;
        self.x2_r = 0.0;
        self.y1_r = 0.0;
        self.y2_r = 0.0;
    }

    /// Set coefficients from the RBJ cookbook. `gain_db` only matters
    /// for `Peak`, `LowShelf`, and `HighShelf`; other kinds ignore it.
    pub fn set(
        &mut self,
        kind: BiquadKind,
        sample_rate: f32,
        cutoff_hz: f32,
        q: f32,
        gain_db: f32,
    ) {
        let sr = sample_rate.max(1.0);
        let cutoff = cutoff_hz.clamp(1.0, sr * 0.499);
        let q = q.max(0.0001);
        let w0 = 2.0 * PI * cutoff / sr;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);
        let a = 10.0_f32.powf(gain_db / 40.0);

        let (b0, b1, b2, a0, a1, a2) = match kind {
            BiquadKind::LowPass => {
                let b0 = (1.0 - cos_w0) * 0.5;
                let b1 = 1.0 - cos_w0;
                let b2 = (1.0 - cos_w0) * 0.5;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            BiquadKind::HighPass => {
                let b0 = (1.0 + cos_w0) * 0.5;
                let b1 = -(1.0 + cos_w0);
                let b2 = (1.0 + cos_w0) * 0.5;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            BiquadKind::BandPass => {
                let b0 = alpha;
                let b1 = 0.0;
                let b2 = -alpha;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            BiquadKind::Notch => {
                let b0 = 1.0;
                let b1 = -2.0 * cos_w0;
                let b2 = 1.0;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            BiquadKind::Peak => {
                let b0 = 1.0 + alpha * a;
                let b1 = -2.0 * cos_w0;
                let b2 = 1.0 - alpha * a;
                let a0 = 1.0 + alpha / a;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha / a;
                (b0, b1, b2, a0, a1, a2)
            }
            BiquadKind::LowShelf => {
                let s = 1.0;
                let beta = (a / q).sqrt();
                let shelf_alpha = sin_w0 / 2.0 * ((a + 1.0 / a) * (1.0 / s - 1.0) + 2.0).sqrt();
                let _ = beta; // kept for clarity; RBJ variant uses alpha directly
                let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * a.sqrt() * shelf_alpha);
                let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
                let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * a.sqrt() * shelf_alpha);
                let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * a.sqrt() * shelf_alpha;
                let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
                let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * a.sqrt() * shelf_alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            BiquadKind::HighShelf => {
                let s = 1.0;
                let shelf_alpha = sin_w0 / 2.0 * ((a + 1.0 / a) * (1.0 / s - 1.0) + 2.0).sqrt();
                let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * a.sqrt() * shelf_alpha);
                let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
                let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * a.sqrt() * shelf_alpha);
                let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * a.sqrt() * shelf_alpha;
                let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
                let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * a.sqrt() * shelf_alpha;
                (b0, b1, b2, a0, a1, a2)
            }
        };

        let inv_a0 = 1.0 / a0;
        self.b0 = b0 * inv_a0;
        self.b1 = b1 * inv_a0;
        self.b2 = b2 * inv_a0;
        self.a1 = a1 * inv_a0;
        self.a2 = a2 * inv_a0;
    }

    /// Mono convenience wrapper. Uses the left-channel state.
    #[inline]
    pub fn process_mono(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1_l + self.b2 * self.x2_l
            - self.a1 * self.y1_l
            - self.a2 * self.y2_l;
        self.x2_l = self.x1_l;
        self.x1_l = x;
        self.y2_l = self.y1_l;
        self.y1_l = y;
        y
    }

    /// Process a stereo frame through independent L/R delay lines with
    /// shared coefficients. Returns `(l, r)`.
    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let yl = self.b0 * l + self.b1 * self.x1_l + self.b2 * self.x2_l
            - self.a1 * self.y1_l
            - self.a2 * self.y2_l;
        self.x2_l = self.x1_l;
        self.x1_l = l;
        self.y2_l = self.y1_l;
        self.y1_l = yl;

        let yr = self.b0 * r + self.b1 * self.x1_r + self.b2 * self.x2_r
            - self.a1 * self.y1_r
            - self.a2 * self.y2_r;
        self.x2_r = self.x1_r;
        self.x1_r = r;
        self.y2_r = self.y1_r;
        self.y1_r = yr;

        (yl, yr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compute the magnitude response of the biquad at a given frequency
    /// by feeding a sinusoid for long enough to settle.
    fn magnitude_at(
        kind: BiquadKind,
        sr: f32,
        cutoff: f32,
        q: f32,
        gain_db: f32,
        test_hz: f32,
    ) -> f32 {
        let mut bq = Biquad::default();
        bq.set(kind, sr, cutoff, q, gain_db);
        let n = (sr * 0.1) as usize; // 100 ms
        let mut peak = 0.0_f32;
        for i in 0..n {
            let phase = 2.0 * std::f32::consts::PI * test_hz * (i as f32) / sr;
            let x = phase.sin();
            let y = bq.process_mono(x);
            if i > n / 2 && y.abs() > peak {
                peak = y.abs();
            }
        }
        peak
    }

    #[test]
    fn low_pass_has_dc_gain_of_one() {
        let mag = magnitude_at(BiquadKind::LowPass, 48_000.0, 1000.0, 0.707, 0.0, 10.0);
        // Very low test frequency (10 Hz) should pass through essentially
        // unchanged.
        assert!((mag - 1.0).abs() < 0.05, "LP DC gain {mag}");
    }

    #[test]
    fn low_pass_attenuates_high_frequencies() {
        let mag = magnitude_at(BiquadKind::LowPass, 48_000.0, 1000.0, 0.707, 0.0, 10_000.0);
        // At 10 kHz with 1 kHz cutoff, we expect heavy attenuation.
        assert!(mag < 0.2, "LP 10kHz mag = {mag}");
    }

    #[test]
    fn high_pass_attenuates_low_frequencies() {
        let mag = magnitude_at(BiquadKind::HighPass, 48_000.0, 1000.0, 0.707, 0.0, 50.0);
        assert!(mag < 0.2, "HP 50Hz mag = {mag}");
    }

    #[test]
    fn high_pass_passes_high_frequencies() {
        let mag = magnitude_at(BiquadKind::HighPass, 48_000.0, 1000.0, 0.707, 0.0, 8000.0);
        assert!((mag - 1.0).abs() < 0.1, "HP 8kHz mag {mag}");
    }

    #[test]
    fn notch_attenuates_at_cutoff() {
        let mag = magnitude_at(BiquadKind::Notch, 48_000.0, 1000.0, 3.0, 0.0, 1000.0);
        assert!(mag < 0.3, "Notch at cutoff mag = {mag}");
    }

    #[test]
    fn peak_boost_at_cutoff() {
        // +6 dB peak at cutoff should roughly double the amplitude.
        let mag = magnitude_at(BiquadKind::Peak, 48_000.0, 1000.0, 1.0, 6.0, 1000.0);
        assert!(mag > 1.7, "Peak +6dB mag = {mag}");
        assert!(mag < 2.3, "Peak +6dB mag overshoot = {mag}");
    }

    #[test]
    fn low_shelf_boosts_low_frequencies() {
        let mag = magnitude_at(BiquadKind::LowShelf, 48_000.0, 500.0, 0.707, 6.0, 50.0);
        // +6 dB shelf below 500 Hz: a 50 Hz tone should come out
        // boosted by ~2x.
        assert!(mag > 1.7, "LowShelf bass boost mag = {mag}");
    }

    #[test]
    fn high_shelf_boosts_high_frequencies() {
        let mag = magnitude_at(
            BiquadKind::HighShelf,
            48_000.0,
            5_000.0,
            0.707,
            6.0,
            12_000.0,
        );
        assert!(mag > 1.7, "HighShelf treble boost mag = {mag}");
    }

    #[test]
    fn reset_clears_state_without_changing_coeffs() {
        let mut bq = Biquad::default();
        bq.set(BiquadKind::LowPass, 48_000.0, 1_000.0, 0.707, 0.0);
        for _ in 0..100 {
            bq.process_mono(1.0);
        }
        bq.reset();
        // First sample after reset with zero input should be 0 — state
        // was cleared.
        let y = bq.process_mono(0.0);
        assert!(y.abs() < 1e-6, "expected clean state after reset, got {y}");
    }
}
