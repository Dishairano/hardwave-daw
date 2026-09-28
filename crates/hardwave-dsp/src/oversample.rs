//! Oversampling for the plug-ins that bend the waveform.
//!
//! Distortion, saturation and clipping all fold harmonics upwards. Anything
//! they generate above half the sample rate has nowhere to go and folds back
//! down as a ringing that is not part of the harmonic series, which is what
//! "digital" harshness usually is. Running the nonlinear part at twice or
//! four times the rate pushes that fold-back above hearing before it comes
//! back down.
//!
//! The filters are cascaded Butterworth low-passes at just under the
//! original Nyquist: cheap, flat in the band that matters, and steep enough
//! that what leaks through is far below the distortion's own noise. A
//! polyphase FIR would be sharper and costs more than this earns.

use crate::biquad::{Biquad, BiquadKind};

/// How much to run the nonlinear part at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OversampleFactor {
    Off,
    Two,
    Four,
}

impl OversampleFactor {
    pub fn ratio(self) -> usize {
        match self {
            OversampleFactor::Off => 1,
            OversampleFactor::Two => 2,
            OversampleFactor::Four => 4,
        }
    }

    pub fn from_u8(v: u8) -> Self {
        match v {
            2 => OversampleFactor::Two,
            4 => OversampleFactor::Four,
            _ => OversampleFactor::Off,
        }
    }
}

/// Runs a per-sample nonlinearity at a higher rate.
///
/// Zero-stuffing to raise the rate, filter, apply the nonlinearity, filter
/// again, and keep one sample in every `ratio`. Stereo, because every
/// plug-in that needs this is stereo.
pub struct Oversampler {
    factor: OversampleFactor,
    sample_rate: f32,
    up: [Biquad; 2],
    down: [Biquad; 2],
}

impl Oversampler {
    pub fn new() -> Self {
        Self {
            factor: OversampleFactor::Off,
            sample_rate: 48_000.0,
            up: [Biquad::default(), Biquad::default()],
            down: [Biquad::default(), Biquad::default()],
        }
    }

    /// Set the rate and the factor. Cheap enough to call per block; the
    /// filters are only rebuilt when something actually changed.
    pub fn configure(&mut self, sample_rate: f32, factor: OversampleFactor) {
        if (sample_rate - self.sample_rate).abs() < 0.5 && factor == self.factor {
            return;
        }
        self.sample_rate = sample_rate.max(1.0);
        self.factor = factor;
        let ratio = factor.ratio() as f32;
        let inner_rate = self.sample_rate * ratio;
        // Just under the original Nyquist: everything the nonlinearity
        // makes above this is what we are here to remove.
        let cutoff = (self.sample_rate * 0.45).min(inner_rate * 0.45);
        // Two identical sections in series: 24 dB per octave, and the pair
        // has no resonant bump at the corner.
        for stage in self.up.iter_mut().chain(self.down.iter_mut()) {
            stage.reset();
            stage.set(
                BiquadKind::LowPass,
                inner_rate,
                cutoff,
                std::f32::consts::FRAC_1_SQRT_2,
                0.0,
            );
        }
    }

    pub fn factor(&self) -> OversampleFactor {
        self.factor
    }

    /// Apply `shape` to one stereo frame at the higher rate.
    ///
    /// With the factor off this is the nonlinearity and nothing else, so a
    /// plug-in can call it unconditionally.
    #[inline]
    pub fn process_frame(
        &mut self,
        l: f32,
        r: f32,
        mut shape: impl FnMut(f32, f32) -> (f32, f32),
    ) -> (f32, f32) {
        let ratio = self.factor.ratio();
        if ratio == 1 {
            return shape(l, r);
        }
        let gain = ratio as f32;
        let mut out = (0.0, 0.0);
        for step in 0..ratio {
            // Zero-stuff: the real sample first, silence after it. The gain
            // puts back what spreading one sample over `ratio` slots takes
            // away.
            let (raw_l, raw_r) = if step == 0 {
                (l * gain, r * gain)
            } else {
                (0.0, 0.0)
            };
            let (up_l, up_r) = self.up[0].process_stereo(raw_l, raw_r);
            let (up_l, up_r) = self.up[1].process_stereo(up_l, up_r);
            let (shaped_l, shaped_r) = shape(up_l, up_r);
            let (down_l, down_r) = self.down[0].process_stereo(shaped_l, shaped_r);
            let (down_l, down_r) = self.down[1].process_stereo(down_l, down_r);
            // Keep the first of each group: the filters have already
            // removed what would alias.
            if step == 0 {
                out = (down_l, down_r);
            }
        }
        out
    }

    pub fn reset(&mut self) {
        for stage in self.up.iter_mut().chain(self.down.iter_mut()) {
            stage.reset();
        }
    }
}

impl Default for Oversampler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Energy above the original Nyquist, as a share of the total. A hard
    /// clipper on a loud sine is the worst case for fold-back.
    fn aliasing_share(factor: OversampleFactor) -> f32 {
        let sample_rate = 48_000.0_f32;
        let freq = 7000.0_f32; // its harmonics land above Nyquist quickly
        let mut os = Oversampler::new();
        os.configure(sample_rate, factor);
        let n = 8192;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let phase = 2.0 * std::f32::consts::PI * freq * (i as f32) / sample_rate;
            let x = phase.sin() * 0.9;
            let (y, _) = os.process_frame(x, x, |l, r| (l.clamp(-0.3, 0.3), r.clamp(-0.3, 0.3)));
            out.push(y);
        }
        // Everything that is not a harmonic of 7 kHz below Nyquist is
        // fold-back. Measured as energy at the frequencies a hard clip
        // cannot legitimately produce: 7 kHz has harmonics at 21 and 35 kHz,
        // which fold to 27 and 13 kHz; the second is audible.
        let fold_back = goertzel(&out, sample_rate, 13_000.0);
        let fundamental = goertzel(&out, sample_rate, freq);
        fold_back / fundamental.max(1e-9)
    }

    /// One frequency's magnitude, without pulling in an FFT.
    fn goertzel(samples: &[f32], sample_rate: f32, freq: f32) -> f32 {
        let k = 2.0 * std::f32::consts::PI * freq / sample_rate;
        let coeff = 2.0 * k.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &x in samples {
            let s0 = x + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt()
    }

    #[test]
    fn oversampling_reduces_the_fold_back_a_clipper_makes() {
        let plain = aliasing_share(OversampleFactor::Off);
        let twice = aliasing_share(OversampleFactor::Two);
        assert!(
            twice < plain * 0.7,
            "2x should cut the fold-back well below the plain rate: {twice} vs {plain}"
        );
    }

    #[test]
    fn with_the_factor_off_the_shape_is_all_that_happens() {
        let mut os = Oversampler::new();
        os.configure(48_000.0, OversampleFactor::Off);
        let (l, r) = os.process_frame(0.8, -0.8, |l, r| (l * 0.5, r * 0.5));
        assert!((l - 0.4).abs() < 1e-6);
        assert!((r + 0.4).abs() < 1e-6);
    }

    #[test]
    fn a_stored_number_picks_the_factor() {
        assert_eq!(OversampleFactor::from_u8(2), OversampleFactor::Two);
        assert_eq!(OversampleFactor::from_u8(4), OversampleFactor::Four);
        assert_eq!(OversampleFactor::from_u8(0), OversampleFactor::Off);
        assert_eq!(OversampleFactor::from_u8(9), OversampleFactor::Off);
    }
}
