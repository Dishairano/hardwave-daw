//! Multiband primitives — LR4 Linkwitz-Riley crossover for splitting
//! a stereo signal into N phase-coherent bands, plus per-band gain
//! control and a simple multiband compressor built on top of the
//! v0.89 dynamics primitives.

use crate::biquad::{Biquad, BiquadKind};
use crate::dynamics::{
    auto_makeup_gain_db, compressor_gain_reduction_db, db_to_linear, linear_to_db, DetectMode,
    EnvelopeFollower,
};

/// Linkwitz-Riley 4th-order split: two cascaded biquads per slope
/// yields the steep 24 dB/oct rolloff with flat summed response.
/// The struct is stateful across calls (per-channel biquad state).
pub struct LR4Split {
    lp1_l: Biquad,
    lp2_l: Biquad,
    hp1_l: Biquad,
    hp2_l: Biquad,
    lp1_r: Biquad,
    lp2_r: Biquad,
    hp1_r: Biquad,
    hp2_r: Biquad,
    crossover_hz: f32,
}

impl LR4Split {
    pub fn new(sample_rate: f32, crossover_hz: f32) -> Self {
        let mut s = Self {
            lp1_l: Biquad::default(),
            lp2_l: Biquad::default(),
            hp1_l: Biquad::default(),
            hp2_l: Biquad::default(),
            lp1_r: Biquad::default(),
            lp2_r: Biquad::default(),
            hp1_r: Biquad::default(),
            hp2_r: Biquad::default(),
            crossover_hz,
        };
        s.set_crossover(sample_rate, crossover_hz);
        s
    }

    pub fn set_crossover(&mut self, sample_rate: f32, crossover_hz: f32) {
        self.crossover_hz = crossover_hz;
        // Butterworth biquads in series = Linkwitz-Riley response.
        let q = 0.707;
        for bq in [
            &mut self.lp1_l,
            &mut self.lp2_l,
            &mut self.lp1_r,
            &mut self.lp2_r,
        ] {
            bq.set(BiquadKind::LowPass, sample_rate, crossover_hz, q, 0.0);
        }
        for bq in [
            &mut self.hp1_l,
            &mut self.hp2_l,
            &mut self.hp1_r,
            &mut self.hp2_r,
        ] {
            bq.set(BiquadKind::HighPass, sample_rate, crossover_hz, q, 0.0);
        }
    }

    pub fn reset(&mut self) {
        for bq in [
            &mut self.lp1_l,
            &mut self.lp2_l,
            &mut self.lp1_r,
            &mut self.lp2_r,
            &mut self.hp1_l,
            &mut self.hp2_l,
            &mut self.hp1_r,
            &mut self.hp2_r,
        ] {
            bq.reset();
        }
    }

    /// Split `(l, r)` into `(low_l, low_r, high_l, high_r)`.
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32, f32, f32) {
        let ll = self.lp2_l.process_mono(self.lp1_l.process_mono(l));
        let lr = self.lp2_r.process_mono(self.lp1_r.process_mono(r));
        let hl = self.hp2_l.process_mono(self.hp1_l.process_mono(l));
        let hr = self.hp2_r.process_mono(self.hp1_r.process_mono(r));
        (ll, lr, hl, hr)
    }
}

/// Per-band compressor parameters for a multiband compressor.
#[derive(Clone, Copy)]
pub struct BandCompressor {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub knee_db: f32,
    pub makeup_db: f32,
    pub bypass: bool,
    pub solo: bool,
}

impl Default for BandCompressor {
    fn default() -> Self {
        Self {
            threshold_db: -12.0,
            ratio: 3.0,
            attack_ms: 5.0,
            release_ms: 50.0,
            knee_db: 6.0,
            makeup_db: 0.0,
            bypass: false,
            solo: false,
        }
    }
}

/// Per-band state — envelope follower plus the compressor parameters.
struct BandState {
    params: BandCompressor,
    env_l: EnvelopeFollower,
    env_r: EnvelopeFollower,
}

impl BandState {
    fn new(sample_rate: f32) -> Self {
        let mut env_l = EnvelopeFollower::default();
        let mut env_r = EnvelopeFollower::default();
        env_l.set_mode(DetectMode::Peak);
        env_r.set_mode(DetectMode::Peak);
        env_l.set_times(5.0, 50.0, sample_rate);
        env_r.set_times(5.0, 50.0, sample_rate);
        Self {
            params: BandCompressor::default(),
            env_l,
            env_r,
        }
    }
}

/// 3-band compressor: low / mid / high, split by two LR4 crossovers.
/// Each band has its own compressor parameters.
pub struct MultibandCompressor3 {
    low_mid_split: LR4Split,
    mid_high_split: LR4Split,
    bands: [BandState; 3],
    output_gain_db: f32,
    sample_rate: f32,
}

impl MultibandCompressor3 {
    pub fn new(sample_rate: f32, low_mid_hz: f32, mid_high_hz: f32) -> Self {
        Self {
            low_mid_split: LR4Split::new(sample_rate, low_mid_hz),
            mid_high_split: LR4Split::new(sample_rate, mid_high_hz),
            bands: [
                BandState::new(sample_rate),
                BandState::new(sample_rate),
                BandState::new(sample_rate),
            ],
            output_gain_db: 0.0,
            sample_rate: sample_rate.max(1.0),
        }
    }

    pub fn set_crossovers(&mut self, low_mid_hz: f32, mid_high_hz: f32) {
        self.low_mid_split
            .set_crossover(self.sample_rate, low_mid_hz);
        self.mid_high_split
            .set_crossover(self.sample_rate, mid_high_hz);
    }

    pub fn band_params_mut(&mut self, band_index: usize) -> Option<&mut BandCompressor> {
        self.bands.get_mut(band_index).map(|b| &mut b.params)
    }

    pub fn set_band_params(&mut self, band_index: usize, params: BandCompressor) {
        if let Some(band) = self.bands.get_mut(band_index) {
            band.env_l
                .set_times(params.attack_ms, params.release_ms, self.sample_rate);
            band.env_r
                .set_times(params.attack_ms, params.release_ms, self.sample_rate);
            band.params = params;
        }
    }

    pub fn set_output_gain_db(&mut self, gain_db: f32) {
        self.output_gain_db = gain_db.clamp(-24.0, 24.0);
    }

    pub fn reset(&mut self) {
        self.low_mid_split.reset();
        self.mid_high_split.reset();
        for band in self.bands.iter_mut() {
            band.env_l.reset();
            band.env_r.reset();
        }
    }

    /// Process one stereo frame through the three-band chain.
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        // Split low from rest.
        let (low_l, low_r, highish_l, highish_r) = self.low_mid_split.process(l, r);
        // Split rest into mid and high.
        let (mid_l, mid_r, hi_l, hi_r) = self.mid_high_split.process(highish_l, highish_r);

        let any_solo = self.bands.iter().any(|b| b.params.solo);

        let (processed_low_l, processed_low_r) = self.process_band(0, low_l, low_r, any_solo);
        let (processed_mid_l, processed_mid_r) = self.process_band(1, mid_l, mid_r, any_solo);
        let (processed_hi_l, processed_hi_r) = self.process_band(2, hi_l, hi_r, any_solo);

        let sum_l = processed_low_l + processed_mid_l + processed_hi_l;
        let sum_r = processed_low_r + processed_mid_r + processed_hi_r;
        let out_gain = db_to_linear(self.output_gain_db);
        (sum_l * out_gain, sum_r * out_gain)
    }

    fn process_band(&mut self, band_index: usize, l: f32, r: f32, any_solo: bool) -> (f32, f32) {
        let band = &mut self.bands[band_index];
        if any_solo && !band.params.solo {
            return (0.0, 0.0);
        }
        if band.params.bypass {
            return (l, r);
        }
        // Envelope follower per channel.
        let env_l = band.env_l.process(l);
        let env_r = band.env_r.process(r);
        let env_db = linear_to_db(env_l.max(env_r));
        let gr_db = compressor_gain_reduction_db(
            env_db,
            band.params.threshold_db,
            band.params.ratio,
            band.params.knee_db,
        );
        let mut gain = db_to_linear(gr_db + band.params.makeup_db);
        // Auto-makeup if the caller set makeup_db to 0 explicitly.
        if band.params.makeup_db == 0.0 {
            let auto = auto_makeup_gain_db(band.params.threshold_db, band.params.ratio);
            gain *= db_to_linear(auto);
        }
        (l * gain, r * gain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lr4_split_bands_are_complementary_in_magnitude() {
        // For a Linkwitz-Riley crossover, each band is heavily
        // attenuated in the opposite half of the spectrum. Measure
        // peak magnitude of each band over a low vs high probe.
        let mut split = LR4Split::new(48_000.0, 1000.0);
        let mut low_peak_lowf = 0.0_f32;
        let mut high_peak_lowf = 0.0_f32;
        // Probe 200 Hz — should be mostly in the low band.
        for i in 0..48_000 {
            let t = i as f32 / 48_000.0;
            let phase = 2.0 * std::f32::consts::PI * 200.0 * t;
            let x = phase.sin();
            let (ll, _lr, hl, _hr) = split.process(x, x);
            if i > 2000 {
                if ll.abs() > low_peak_lowf {
                    low_peak_lowf = ll.abs();
                }
                if hl.abs() > high_peak_lowf {
                    high_peak_lowf = hl.abs();
                }
            }
        }
        assert!(
            low_peak_lowf > 0.7,
            "low band should pass 200 Hz, got {low_peak_lowf}"
        );
        assert!(
            high_peak_lowf < 0.15,
            "high band should reject 200 Hz, got {high_peak_lowf}"
        );
    }

    #[test]
    fn lr4_low_band_attenuates_highs() {
        let mut split = LR4Split::new(48_000.0, 1000.0);
        let mut max_l = 0.0_f32;
        for i in 0..48_000 {
            let t = i as f32 / 48_000.0;
            let phase = 2.0 * std::f32::consts::PI * 8000.0 * t;
            let x = phase.sin();
            let (ll, _lr, _hl, _hr) = split.process(x, x);
            if i > 1000 && ll.abs() > max_l {
                max_l = ll.abs();
            }
        }
        // 8 kHz through 1 kHz LR4 LP should be heavily attenuated.
        assert!(max_l < 0.1, "low band at 8 kHz = {max_l}");
    }

    #[test]
    fn multiband_compressor_with_bypass_sums_to_similar_magnitude() {
        // With all bands bypassed, output should have similar peak
        // magnitude to input. LR4 sums are magnitude-flat but
        // phase-shifted, so time-domain equality is not preserved.
        let mut mb = MultibandCompressor3::new(48_000.0, 250.0, 4000.0);
        for i in 0..3 {
            if let Some(params) = mb.band_params_mut(i) {
                params.bypass = true;
            }
        }
        let mut max_out = 0.0_f32;
        let input_peak = 0.5_f32;
        for i in 0..48_000 {
            let t = i as f32 / 48_000.0;
            let phase = 2.0 * std::f32::consts::PI * 1000.0 * t;
            let x = input_peak * phase.sin();
            let (ol, _or) = mb.process(x, x);
            if i > 2000 && ol.abs() > max_out {
                max_out = ol.abs();
            }
        }
        // Output peak should be within 50% of input peak — LR4 is
        // magnitude-flat but the 1 kHz probe sits right at the
        // low/mid crossover of 250 Hz so flatness is solid away from
        // crossovers.
        assert!(
            (max_out - input_peak).abs() < 0.15,
            "all-bypass peak {max_out} should track input peak {input_peak}"
        );
    }

    #[test]
    fn multiband_solo_isolates_one_band() {
        let mut mb = MultibandCompressor3::new(48_000.0, 250.0, 4000.0);
        // Solo the low band.
        for i in 0..3 {
            if let Some(params) = mb.band_params_mut(i) {
                params.solo = i == 0;
                params.bypass = true;
            }
        }
        // Feed a 5 kHz tone — the low band is silent, so with only low
        // soloed, output should be near zero.
        let mut max_out = 0.0_f32;
        for i in 0..48_000 {
            let t = i as f32 / 48_000.0;
            let phase = 2.0 * std::f32::consts::PI * 5000.0 * t;
            let x = phase.sin();
            let (ol, _or) = mb.process(x, x);
            if i > 2000 && ol.abs() > max_out {
                max_out = ol.abs();
            }
        }
        assert!(
            max_out < 0.1,
            "solo low band on 5 kHz should be near silent, got {max_out}"
        );
    }

    #[test]
    fn multiband_output_gain_applies_linearly() {
        let mut mb = MultibandCompressor3::new(48_000.0, 250.0, 4000.0);
        for i in 0..3 {
            if let Some(params) = mb.band_params_mut(i) {
                params.bypass = true;
            }
        }
        // +6 dB output gain should roughly double the output amplitude.
        mb.set_output_gain_db(6.0);
        let mut max_out = 0.0_f32;
        for i in 0..48_000 {
            let t = i as f32 / 48_000.0;
            let phase = 2.0 * std::f32::consts::PI * 1000.0 * t;
            let x = 0.5 * phase.sin();
            let (ol, _or) = mb.process(x, x);
            if i > 2000 && ol.abs() > max_out {
                max_out = ol.abs();
            }
        }
        // With bypass and +6 dB, peak ≈ 0.5 × 2 = 1.0.
        assert!(max_out > 0.8 && max_out < 1.2, "+6 dB max out = {max_out}");
    }
}
