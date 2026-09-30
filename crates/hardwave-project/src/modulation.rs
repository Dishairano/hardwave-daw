//! Modulation: an LFO wired to a plug-in's knob.
//!
//! Automation draws a shape once and plays it back. Modulation is the
//! other thing every synth has: a source that keeps running and a knob
//! that follows it, so a filter breathes for the whole song without a
//! single point being drawn.
//!
//! The matrix is a list of routes. Each one says which source, which
//! knob, how far it swings and what it swings around.

use serde::{Deserialize, Serialize};

use crate::lfo::{sample_shape, LfoRate, LfoShape};
use crate::track::TrackId;

/// Where the movement comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ModSource {
    /// A shape that keeps running with the song. Free of the transport's
    /// start point, so the same bar always sounds the same.
    Lfo {
        shape: LfoShape,
        rate: LfoRate,
        /// Where in the cycle the song's start sits, 0 to 1.
        phase_offset: f64,
    },
}

/// What moves.
///
/// Only a plug-in's own parameters for now: the track fader and pan are
/// written every block from their automation lanes, so a route to them
/// would be overwritten each time rather than heard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ModTarget {
    PluginParam { slot_id: String, param_id: u32 },
}

/// One wire in the matrix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModRoute {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub source: ModSource,
    pub track_id: TrackId,
    pub target: ModTarget,
    /// What the knob sits at with the source in the middle, 0 to 1.
    pub center: f64,
    /// How far either side of the centre it swings, 0 to 1. Negative
    /// turns the shape upside down.
    pub depth: f64,
}

impl ModRoute {
    /// Where this route puts its knob at a point in the song.
    ///
    /// Returns 0 to 1, the shape a plug-in parameter takes.
    pub fn value_at(&self, position_ticks: u64, bpm: f64, ppq: u64) -> f64 {
        let swing = match &self.source {
            ModSource::Lfo {
                shape,
                rate,
                phase_offset,
            } => {
                let cycle = rate.cycle_length_ticks(bpm, ppq).max(1);
                let phase =
                    (position_ticks % cycle) as f64 / cycle as f64 + phase_offset.rem_euclid(1.0);
                // sample_shape gives 0..1; the matrix works either side of
                // the centre, so it is moved to -1..1 here.
                sample_shape(*shape, phase, self.id.len() as u64) * 2.0 - 1.0
            }
        };
        (self.center + self.depth * swing).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(depth: f64, center: f64) -> ModRoute {
        ModRoute {
            id: "r1".into(),
            name: "Filter".into(),
            enabled: true,
            source: ModSource::Lfo {
                shape: LfoShape::Sine,
                rate: LfoRate::TempoSync { num: 1, den: 4 },
                phase_offset: 0.0,
            },
            track_id: "t1".into(),
            target: ModTarget::PluginParam {
                slot_id: "s1".into(),
                param_id: 3,
            },
            center,
            depth,
        }
    }

    const PPQ: u64 = 960;

    #[test]
    fn a_route_with_no_depth_sits_still_at_its_centre() {
        let r = route(0.0, 0.7);
        for tick in [0, 240, 480, 960, 1920] {
            assert!((r.value_at(tick, 140.0, PPQ) - 0.7).abs() < 1e-9);
        }
    }

    #[test]
    fn a_sine_swings_either_side_of_the_centre() {
        let r = route(0.4, 0.5);
        // A quarter-note cycle at 960 ppq: the sine's top is a quarter of
        // the way through, its bottom three quarters of the way.
        let top = r.value_at(PPQ / 4, 140.0, PPQ);
        let bottom = r.value_at(PPQ * 3 / 4, 140.0, PPQ);
        assert!(top > 0.85, "top of the swing, got {top}");
        assert!(bottom < 0.15, "bottom of the swing, got {bottom}");
    }

    #[test]
    fn the_knob_never_goes_past_its_ends() {
        let r = route(1.0, 0.9);
        for tick in 0..PPQ {
            let v = r.value_at(tick, 140.0, PPQ);
            assert!((0.0..=1.0).contains(&v), "value {v} at tick {tick}");
        }
    }

    #[test]
    fn a_negative_depth_turns_the_shape_upside_down() {
        let up = route(0.4, 0.5);
        let down = route(-0.4, 0.5);
        let tick = PPQ / 4;
        let a = up.value_at(tick, 140.0, PPQ);
        let b = down.value_at(tick, 140.0, PPQ);
        assert!((a + b - 1.0).abs() < 1e-6, "{a} and {b} should mirror");
    }

    #[test]
    fn the_same_bar_sounds_the_same_however_you_get_there() {
        let r = route(0.5, 0.5);
        // One bar later is the same point in a quarter-note cycle.
        assert!((r.value_at(240, 140.0, PPQ) - r.value_at(240 + PPQ * 4, 140.0, PPQ)).abs() < 1e-9);
    }
}
