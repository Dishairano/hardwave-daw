//! Pan laws.
//!
//! How loud a track stays as it is panned depends on the law the DAW uses.
//! Hardwave had one fixed law, constant power, hard-coded in two places.
//! That is a defensible default and the wrong answer for anyone bringing a
//! song over from a DAW set to something else: the same pan positions come
//! out louder or quieter in the middle, and the mix shifts.

use std::f32::consts::FRAC_PI_2;

/// How much a hard-centred signal is attenuated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanLaw {
    /// Constant power. The centre sits 3 dB down. Hardwave's default, and
    /// the usual choice for music.
    Minus3dB,
    /// Halfway between constant power and linear. Pro Tools' default.
    Minus4_5dB,
    /// Linear taper: the centre sits 6 dB down. Steady when a stereo pair
    /// is summed to mono.
    Minus6dB,
    /// No attenuation at the centre; the far side is boosted instead.
    Zero,
}

impl PanLaw {
    /// The law behind a stored number, so the setting can live in an atomic.
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => PanLaw::Minus4_5dB,
            2 => PanLaw::Minus6dB,
            3 => PanLaw::Zero,
            _ => PanLaw::Minus3dB,
        }
    }

    pub fn as_u8(self) -> u8 {
        match self {
            PanLaw::Minus3dB => 0,
            PanLaw::Minus4_5dB => 1,
            PanLaw::Minus6dB => 2,
            PanLaw::Zero => 3,
        }
    }
}

/// Left and right gain for a pan position, where -1 is hard left, 0 centre
/// and 1 hard right.
#[inline]
pub fn gains(pan: f32, law: PanLaw) -> (f32, f32) {
    let p = ((pan + 1.0) * 0.5).clamp(0.0, 1.0);
    match law {
        PanLaw::Minus3dB => ((p * FRAC_PI_2).cos(), (p * FRAC_PI_2).sin()),
        PanLaw::Minus6dB => (1.0 - p, p),
        PanLaw::Minus4_5dB => {
            // The geometric mean of the two above, which is what -4.5 dB
            // means in practice: 0.5 and 0.707 average to about 0.595.
            let (cl, cr) = ((p * FRAC_PI_2).cos(), (p * FRAC_PI_2).sin());
            (((1.0 - p) * cl).sqrt(), (p * cr).sqrt())
        }
        PanLaw::Zero => ((2.0 * (1.0 - p)).min(1.0), (2.0 * p).min(1.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(gain: f32) -> f32 {
        20.0 * gain.max(1e-9).log10()
    }

    #[test]
    fn each_law_puts_the_centre_where_its_name_says() {
        for (law, expected) in [
            (PanLaw::Minus3dB, -3.0),
            (PanLaw::Minus4_5dB, -4.5),
            (PanLaw::Minus6dB, -6.0),
            (PanLaw::Zero, 0.0),
        ] {
            let (l, r) = gains(0.0, law);
            assert!((l - r).abs() < 1e-6, "{law:?} is not centred");
            assert!(
                (db(l) - expected).abs() < 0.2,
                "{law:?} centre is {} dB, expected {expected}",
                db(l)
            );
        }
    }

    #[test]
    fn hard_left_is_silent_on_the_right_in_every_law() {
        for law in [
            PanLaw::Minus3dB,
            PanLaw::Minus4_5dB,
            PanLaw::Minus6dB,
            PanLaw::Zero,
        ] {
            let (l, r) = gains(-1.0, law);
            assert!(r.abs() < 1e-6, "{law:?} leaks to the right");
            assert!((l - 1.0).abs() < 1e-6, "{law:?} does not reach unity left");
        }
    }

    #[test]
    fn a_stored_number_round_trips() {
        for law in [
            PanLaw::Minus3dB,
            PanLaw::Minus4_5dB,
            PanLaw::Minus6dB,
            PanLaw::Zero,
        ] {
            assert_eq!(PanLaw::from_u8(law.as_u8()), law);
        }
        assert_eq!(
            PanLaw::from_u8(200),
            PanLaw::Minus3dB,
            "an unknown value falls back to the default"
        );
    }
}
