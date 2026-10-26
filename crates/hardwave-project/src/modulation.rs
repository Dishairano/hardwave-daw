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
    /// How loud another track is, followed: it rises at once with the
    /// track and falls back over `release_ms`. A kick track ducking a bass
    /// is this source on the bass's volume.
    Envelope { track_id: TrackId, release_ms: f64 },
    /// A controller on the keyboard or desk (the mod wheel is 1), on any
    /// channel: where it was last moved, 0 to 1.
    MidiCc { cc: u8 },
}

/// What moves.
///
/// A plug-in's knob is set to the route's value. The track's volume, pan
/// and sends are moved on top of what the fader, the pan knob, the send
/// level and their automation say, since those are written every block:
/// volume and a send are multiplied by the value (1 leaves them as they
/// are), and pan is moved by it (0.5 leaves it where it is).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ModTarget {
    PluginParam { slot_id: String, param_id: u32 },
    TrackVolume,
    TrackPan,
    SendLevel { send_index: usize },
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
    /// Where this route puts its knob at a point in the song, for an LFO.
    /// The other sources need what is playing (see `value_with`).
    ///
    /// Returns 0 to 1, the shape a plug-in parameter takes.
    pub fn value_at(&self, position_ticks: u64, bpm: f64, ppq: u64) -> f64 {
        self.value_with(self.lfo_swing(position_ticks, bpm, ppq).unwrap_or(0.0))
    }

    /// An LFO's place in its swing, -1 to 1; None for another source.
    pub fn lfo_swing(&self, position_ticks: u64, bpm: f64, ppq: u64) -> Option<f64> {
        match &self.source {
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
                Some(sample_shape(*shape, phase, self.id.len() as u64) * 2.0 - 1.0)
            }
            _ => None,
        }
    }

    /// The route's value for a source at `swing` (-1 to 1): the centre,
    /// moved by the depth either side. A level or a controller from 0 to 1
    /// is `2 * x - 1` here, so at rest it sits at the centre minus the
    /// depth.
    pub fn value_with(&self, swing: f64) -> f64 {
        (self.center + self.depth * swing.clamp(-1.0, 1.0)).clamp(0.0, 1.0)
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

    /// A route to the volume ducking from a kick: at rest the bass plays
    /// as set (1), at the kick's peak it is halved (0.5).
    #[test]
    fn an_envelope_route_ducks_from_its_rest() {
        let duck = ModRoute {
            source: ModSource::Envelope {
                track_id: "kick".into(),
                release_ms: 120.0,
            },
            target: ModTarget::TrackVolume,
            ..route(-0.25, 0.75)
        };
        assert!(
            (duck.value_with(-1.0) - 1.0).abs() < 1e-9,
            "silence: untouched"
        );
        assert!(
            (duck.value_with(1.0) - 0.5).abs() < 1e-9,
            "full kick: halved"
        );
        assert_eq!(duck.lfo_swing(0, 140.0, PPQ), None);
    }
}
