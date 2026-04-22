//! 7-band parametric EQ — low shelf + five parametric peaks + high
//! shelf. Each band composes a `Biquad` and exposes frequency / gain
//! / Q / type / enable controls. Matches the standard "surgical EQ"
//! feature set most DAWs ship.

use crate::biquad::{Biquad, BiquadKind};

/// Per-band filter kind. LowShelf / Peak / HighShelf are the default
/// shapes, but each band can be switched to notch or LP/HP for
/// broader surgical work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqBandKind {
    LowShelf,
    HighShelf,
    Peak,
    Notch,
    LowPass,
    HighPass,
}

impl EqBandKind {
    fn to_biquad_kind(self) -> BiquadKind {
        match self {
            EqBandKind::LowShelf => BiquadKind::LowShelf,
            EqBandKind::HighShelf => BiquadKind::HighShelf,
            EqBandKind::Peak => BiquadKind::Peak,
            EqBandKind::Notch => BiquadKind::Notch,
            EqBandKind::LowPass => BiquadKind::LowPass,
            EqBandKind::HighPass => BiquadKind::HighPass,
        }
    }
}

/// One EQ band with its own `Biquad`, enable flag, and solo state.
pub struct EqBand {
    biquad: Biquad,
    enabled: bool,
    frequency_hz: f32,
    gain_db: f32,
    q: f32,
    kind: EqBandKind,
    solo: bool,
}

impl EqBand {
    pub fn new(kind: EqBandKind, frequency_hz: f32) -> Self {
        Self {
            biquad: Biquad::default(),
            enabled: true,
            frequency_hz,
            gain_db: 0.0,
            q: 0.707,
            kind,
            solo: false,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_solo(&mut self, solo: bool) {
        self.solo = solo;
    }

    pub fn is_solo(&self) -> bool {
        self.solo
    }

    pub fn set_frequency(&mut self, hz: f32) {
        self.frequency_hz = hz.clamp(20.0, 20_000.0);
    }

    pub fn set_gain_db(&mut self, gain_db: f32) {
        self.gain_db = gain_db.clamp(-24.0, 24.0);
    }

    pub fn set_q(&mut self, q: f32) {
        self.q = q.clamp(0.1, 10.0);
    }

    pub fn set_kind(&mut self, kind: EqBandKind) {
        self.kind = kind;
    }

    pub fn update_coeffs(&mut self, sample_rate: f32) {
        self.biquad.set(
            self.kind.to_biquad_kind(),
            sample_rate,
            self.frequency_hz,
            self.q,
            self.gain_db,
        );
    }

    pub fn reset(&mut self) {
        self.biquad.reset();
    }

    #[inline]
    pub fn process_mono(&mut self, x: f32) -> f32 {
        if self.enabled {
            self.biquad.process_mono(x)
        } else {
            x
        }
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.enabled {
            self.biquad.process_stereo(l, r)
        } else {
            (l, r)
        }
    }
}

/// 7-band parametric EQ — 1 low shelf, 5 peaks, 1 high shelf by
/// default, but each band's kind can be changed at runtime.
pub struct ParametricEq7 {
    bands: [EqBand; 7],
    sample_rate: f32,
}

impl ParametricEq7 {
    pub fn new(sample_rate: f32) -> Self {
        let default_freqs = [60.0, 120.0, 400.0, 1000.0, 3000.0, 7000.0, 12000.0];
        let default_kinds = [
            EqBandKind::LowShelf,
            EqBandKind::Peak,
            EqBandKind::Peak,
            EqBandKind::Peak,
            EqBandKind::Peak,
            EqBandKind::Peak,
            EqBandKind::HighShelf,
        ];
        let mut bands = std::array::from_fn(|i| EqBand::new(default_kinds[i], default_freqs[i]));
        for band in bands.iter_mut() {
            band.update_coeffs(sample_rate);
        }
        Self {
            bands,
            sample_rate: sample_rate.max(1.0),
        }
    }

    pub fn band_mut(&mut self, index: usize) -> Option<&mut EqBand> {
        self.bands.get_mut(index)
    }

    /// Recompute all enabled bands' coefficients after parameter
    /// changes. Call after `set_frequency` / `set_gain_db` / `set_q`
    /// / `set_kind` on any band.
    pub fn update_all_coeffs(&mut self) {
        let sr = self.sample_rate;
        for band in self.bands.iter_mut() {
            band.update_coeffs(sr);
        }
    }

    pub fn reset(&mut self) {
        for band in self.bands.iter_mut() {
            band.reset();
        }
    }

    /// Process a mono sample through all 7 bands in series.
    /// When any band is soloed, only soloed bands are active.
    pub fn process_mono(&mut self, x: f32) -> f32 {
        let any_solo = self.bands.iter().any(|b| b.solo);
        let mut v = x;
        for band in self.bands.iter_mut() {
            if any_solo && !band.solo {
                continue;
            }
            v = band.process_mono(v);
        }
        v
    }

    /// Process a stereo frame through all 7 bands in series.
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let any_solo = self.bands.iter().any(|b| b.solo);
        let mut lv = l;
        let mut rv = r;
        for band in self.bands.iter_mut() {
            if any_solo && !band.solo {
                continue;
            }
            let (nl, nr) = band.process_stereo(lv, rv);
            lv = nl;
            rv = nr;
        }
        (lv, rv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn magnitude_at(eq: &mut ParametricEq7, hz: f32) -> f32 {
        let sr = 48_000.0;
        let n = (sr * 0.1) as usize;
        let mut max = 0.0_f32;
        for i in 0..n {
            let phase = 2.0 * std::f32::consts::PI * hz * (i as f32) / sr;
            let x = phase.sin();
            let y = eq.process_mono(x);
            if i > n / 2 && y.abs() > max {
                max = y.abs();
            }
        }
        max
    }

    #[test]
    fn seven_band_default_is_identity() {
        let mut eq = ParametricEq7::new(48_000.0);
        // Default bands have 0 dB gain — magnitude should stay ≈ 1.0.
        let mag = magnitude_at(&mut eq, 1000.0);
        assert!((mag - 1.0).abs() < 0.1, "identity mag = {mag}");
    }

    #[test]
    fn peak_band_boosts_at_center_frequency() {
        let mut eq = ParametricEq7::new(48_000.0);
        if let Some(band) = eq.band_mut(3) {
            band.set_frequency(1000.0);
            band.set_gain_db(6.0);
            band.set_q(1.0);
        }
        eq.update_all_coeffs();
        let mag_on = magnitude_at(&mut eq, 1000.0);
        // +6 dB is roughly 2× amplitude.
        assert!(mag_on > 1.7, "peak boost at 1 kHz = {mag_on}");
    }

    #[test]
    fn peak_band_cut_attenuates_at_center_frequency() {
        let mut eq = ParametricEq7::new(48_000.0);
        if let Some(band) = eq.band_mut(3) {
            band.set_frequency(1000.0);
            band.set_gain_db(-12.0);
            band.set_q(1.0);
        }
        eq.update_all_coeffs();
        let mag_on = magnitude_at(&mut eq, 1000.0);
        // -12 dB ≈ 0.25× amplitude.
        assert!(mag_on < 0.35, "peak cut at 1 kHz = {mag_on}");
    }

    #[test]
    fn disabled_band_bypasses_processing() {
        let mut eq = ParametricEq7::new(48_000.0);
        if let Some(band) = eq.band_mut(3) {
            band.set_gain_db(24.0);
            band.set_enabled(false);
        }
        eq.update_all_coeffs();
        let mag = magnitude_at(&mut eq, 1000.0);
        // Disabled band should pass through cleanly.
        assert!((mag - 1.0).abs() < 0.1, "disabled band mag = {mag}");
    }

    #[test]
    fn solo_band_isolates_its_contribution() {
        let mut eq = ParametricEq7::new(48_000.0);
        // Set all bands to heavy boosts.
        for i in 0..7 {
            if let Some(band) = eq.band_mut(i) {
                band.set_gain_db(12.0);
            }
        }
        // Solo the 1 kHz peak band.
        if let Some(band) = eq.band_mut(3) {
            band.set_solo(true);
        }
        eq.update_all_coeffs();
        let mag = magnitude_at(&mut eq, 1000.0);
        // Only band 3 is active — +12 dB at 1 kHz ≈ 4× amplitude.
        assert!(mag > 2.5 && mag < 5.0, "solo +12 dB mag = {mag}");
    }

    #[test]
    fn band_kind_can_change_at_runtime() {
        let mut eq = ParametricEq7::new(48_000.0);
        if let Some(band) = eq.band_mut(3) {
            band.set_kind(EqBandKind::Notch);
            band.set_frequency(1000.0);
            band.set_q(5.0);
        }
        eq.update_all_coeffs();
        let mag = magnitude_at(&mut eq, 1000.0);
        // Narrow notch at the probe frequency should heavily attenuate.
        assert!(mag < 0.3, "notch mag = {mag}");
    }

    #[test]
    fn low_shelf_boosts_bass() {
        let mut eq = ParametricEq7::new(48_000.0);
        if let Some(band) = eq.band_mut(0) {
            band.set_gain_db(6.0);
            band.set_frequency(200.0);
        }
        eq.update_all_coeffs();
        let mag = magnitude_at(&mut eq, 50.0);
        assert!(mag > 1.7, "low shelf at 50 Hz = {mag}");
    }

    #[test]
    fn high_shelf_boosts_treble() {
        let mut eq = ParametricEq7::new(48_000.0);
        if let Some(band) = eq.band_mut(6) {
            band.set_gain_db(6.0);
            band.set_frequency(5000.0);
        }
        eq.update_all_coeffs();
        let mag = magnitude_at(&mut eq, 12_000.0);
        assert!(mag > 1.7, "high shelf at 12 kHz = {mag}");
    }
}
