//! Velocity curves for hardware MIDI input.
//!
//! The setup wizard has asked people to pick a velocity curve per controller
//! since it was written, stored the answer, and then never applied it: no code
//! outside the wizard's own store ever read it. A pad controller that plays too
//! hard stayed too hard whatever was chosen.
//!
//! A curve maps an incoming velocity to a played one. Every curve here keeps
//! the ends fixed, so the softest hit a controller can send stays the softest
//! and its hardest stays the hardest, and is monotonic, so harder always means
//! louder. Only the middle moves.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VelocityCurve {
    /// What the controller sent, untouched.
    #[default]
    Linear,
    /// Lifts light hits, for a controller that needs to be hit hard.
    Soft,
    /// Holds light hits down, for a controller that shouts at a light touch.
    Hard,
    /// Pushes hits away from the middle: light hits lighter, hard hits
    /// harder, which is the usual "more dynamics" answer.
    SCurve,
}

impl VelocityCurve {
    /// Parse the id the UI stores, e.g. "s-curve".
    ///
    /// Anything unknown is linear rather than an error: a stored id from an
    /// older build should not stop a controller from playing.
    pub fn from_id(id: &str) -> Self {
        match id {
            "soft" => Self::Soft,
            "hard" => Self::Hard,
            "s-curve" => Self::SCurve,
            _ => Self::Linear,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Soft => "soft",
            Self::Hard => "hard",
            Self::SCurve => "s-curve",
        }
    }

    /// Index used to share the setting with the midir callback through an
    /// atomic, so changing the curve applies to a port that is already open.
    pub fn to_index(&self) -> u8 {
        match self {
            Self::Linear => 0,
            Self::Soft => 1,
            Self::Hard => 2,
            Self::SCurve => 3,
        }
    }

    pub fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Soft,
            2 => Self::Hard,
            3 => Self::SCurve,
            _ => Self::Linear,
        }
    }

    /// Map an incoming velocity in 0.0..=1.0 to the played one.
    pub fn apply(&self, velocity: f32) -> f32 {
        let v = velocity.clamp(0.0, 1.0);
        let shaped = match self {
            Self::Linear => v,
            // A square root lifts the quiet half without touching the ends.
            Self::Soft => v.sqrt(),
            Self::Hard => v * v,
            // Smoothstep: flat at both ends, steep through the middle.
            Self::SCurve => v * v * (3.0 - 2.0 * v),
        };
        shaped.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURVES: [VelocityCurve; 4] = [
        VelocityCurve::Linear,
        VelocityCurve::Soft,
        VelocityCurve::Hard,
        VelocityCurve::SCurve,
    ];

    #[test]
    fn every_curve_keeps_the_softest_and_hardest_hits() {
        for curve in CURVES {
            assert_eq!(curve.apply(0.0), 0.0, "{curve:?} moved silence");
            assert!(
                (curve.apply(1.0) - 1.0).abs() < 1e-6,
                "{curve:?} moved a full hit"
            );
        }
    }

    #[test]
    fn harder_always_means_louder() {
        for curve in CURVES {
            let mut previous = -1.0;
            for step in 0..=127 {
                let out = curve.apply(step as f32 / 127.0);
                assert!(
                    out > previous || (out - previous).abs() < 1e-9,
                    "{curve:?} dipped at step {step}: {previous} then {out}"
                );
                previous = out;
            }
        }
    }

    #[test]
    fn soft_lifts_a_light_hit_and_hard_holds_it_down() {
        let light = 0.25;
        assert!(VelocityCurve::Soft.apply(light) > light);
        assert!(VelocityCurve::Hard.apply(light) < light);
        assert_eq!(VelocityCurve::Linear.apply(light), light);
    }

    #[test]
    fn the_s_curve_widens_the_gap_between_light_and_hard() {
        let light = VelocityCurve::SCurve.apply(0.25);
        let heavy = VelocityCurve::SCurve.apply(0.75);
        assert!(light < 0.25, "light hits should get lighter, got {light}");
        assert!(heavy > 0.75, "hard hits should get harder, got {heavy}");
        // The midpoint is the one place it does not move.
        assert!((VelocityCurve::SCurve.apply(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn an_unknown_id_plays_the_controller_as_it_is() {
        assert_eq!(VelocityCurve::from_id("custom"), VelocityCurve::Linear);
        assert_eq!(VelocityCurve::from_id(""), VelocityCurve::Linear);
        for curve in CURVES {
            assert_eq!(VelocityCurve::from_id(curve.id()), curve, "{curve:?}");
        }
    }

    #[test]
    fn a_curve_survives_the_trip_through_an_atomic() {
        for curve in CURVES {
            assert_eq!(VelocityCurve::from_index(curve.to_index()), curve);
        }
        assert_eq!(VelocityCurve::from_index(99), VelocityCurve::Linear);
    }

    #[test]
    fn a_velocity_outside_the_range_is_clamped() {
        assert_eq!(VelocityCurve::Soft.apply(-1.0), 0.0);
        assert_eq!(VelocityCurve::Hard.apply(2.0), 1.0);
    }
}
