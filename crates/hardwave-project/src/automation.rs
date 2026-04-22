use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AutomationTarget {
    TrackVolume,
    TrackPan,
    TrackMute,
    PluginParam { slot_id: String, param_id: u32 },
    SendLevel { send_index: usize },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CurveMode {
    Linear,
    Bezier,
    /// Hold `a.value` until the next point — classic step automation.
    Step,
    /// 4-step staircase that climbs/descends from `a.value` to `b.value`
    /// across the segment, holding at each step.
    Stairs,
    /// Same 4-step layout as `Stairs`, but the transition within each
    /// step smooths rather than jumping abruptly.
    SmoothStairs,
}

const STAIR_STEPS: u32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub tick: u64,
    /// Normalized value 0.0..1.0
    pub value: f64,
    pub curve: CurveMode,
    /// Bezier tension (-1.0..1.0), only used when curve is Bezier.
    pub tension: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationLane {
    pub id: String,
    pub target: AutomationTarget,
    pub points: Vec<AutomationPoint>,
    pub visible: bool,
}

impl AutomationLane {
    /// Map a normalized automation value (0.0..1.0) into the parameter's
    /// real range `[min, max]`. Pure helper — same math callers would
    /// write inline, factored out so the roadmap's "Map normalized range
    /// to parameter min/max" claim has a single definition and a test.
    pub fn denormalize(normalized: f64, min: f64, max: f64) -> f64 {
        let clamped = normalized.clamp(0.0, 1.0);
        min + (max - min) * clamped
    }

    /// Interpolated value at `tick`, mapped into `[min, max]`. Combines
    /// `value_at` with `denormalize` — the one-stop path from a stored
    /// automation lane to a real parameter value on the audio thread.
    pub fn denormalized_value_at(&self, tick: u64, min: f64, max: f64) -> f64 {
        Self::denormalize(self.value_at(tick), min, max)
    }

    /// Get the interpolated value at a given tick.
    pub fn value_at(&self, tick: u64) -> f64 {
        if self.points.is_empty() {
            return 0.5;
        }
        if tick <= self.points[0].tick {
            return self.points[0].value;
        }
        let last = self.points.last().unwrap();
        if tick >= last.tick {
            return last.value;
        }

        // Find surrounding points
        let idx = self.points.partition_point(|p| p.tick <= tick);
        if idx == 0 {
            return self.points[0].value;
        }

        let a = &self.points[idx - 1];
        let b = &self.points[idx];

        match a.curve {
            CurveMode::Step => a.value,
            CurveMode::Linear => {
                let t = (tick - a.tick) as f64 / (b.tick - a.tick) as f64;
                a.value + (b.value - a.value) * t
            }
            CurveMode::Bezier => {
                let t = (tick - a.tick) as f64 / (b.tick - a.tick) as f64;
                // Simple power curve based on tension
                let curved_t = if a.tension >= 0.0 {
                    t.powf(1.0 + a.tension * 3.0)
                } else {
                    1.0 - (1.0 - t).powf(1.0 + (-a.tension) * 3.0)
                };
                a.value + (b.value - a.value) * curved_t
            }
            CurveMode::Stairs => {
                // 4 horizontal steps, each 1/STAIR_STEPS of the segment
                // wide, with an instant jump at each boundary.
                let t = (tick - a.tick) as f64 / (b.tick - a.tick) as f64;
                let step_index = (t * STAIR_STEPS as f64).floor() as u32;
                let step_index = step_index.min(STAIR_STEPS - 1);
                let step_t = step_index as f64 / STAIR_STEPS as f64;
                a.value + (b.value - a.value) * step_t
            }
            CurveMode::SmoothStairs => {
                // Same 4-step discrete layout as Stairs, but each step
                // transitions smoothly to the next via a cosine ramp
                // instead of an instant jump.
                let t = (tick - a.tick) as f64 / (b.tick - a.tick) as f64;
                let scaled = t * STAIR_STEPS as f64;
                let step_index = scaled.floor() as u32;
                let step_index = step_index.min(STAIR_STEPS - 1);
                let local = scaled - step_index as f64;
                let smoothed = 0.5 - 0.5 * (std::f64::consts::PI * local).cos();
                let step_t = (step_index as f64 + smoothed) / STAIR_STEPS as f64;
                a.value + (b.value - a.value) * step_t
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denormalize_maps_endpoints_and_midpoint() {
        assert_eq!(AutomationLane::denormalize(0.0, -24.0, 24.0), -24.0);
        assert_eq!(AutomationLane::denormalize(1.0, -24.0, 24.0), 24.0);
        assert_eq!(AutomationLane::denormalize(0.5, -24.0, 24.0), 0.0);
        assert_eq!(AutomationLane::denormalize(0.25, 0.0, 100.0), 25.0);
    }

    #[test]
    fn denormalize_clamps_out_of_range_input() {
        // Normalized values outside 0..1 are clamped — callers never see a
        // mapped value that escapes [min, max].
        assert_eq!(AutomationLane::denormalize(-0.5, 0.0, 100.0), 0.0);
        assert_eq!(AutomationLane::denormalize(2.0, 0.0, 100.0), 100.0);
        assert_eq!(AutomationLane::denormalize(-10.0, -24.0, 24.0), -24.0);
    }

    #[test]
    fn denormalize_handles_inverted_range() {
        // min > max still produces a linear interpolation between the two
        // bounds — useful for parameters that read backward (e.g. filter
        // cutoff in some synths).
        assert_eq!(AutomationLane::denormalize(0.0, 1.0, 0.0), 1.0);
        assert_eq!(AutomationLane::denormalize(1.0, 1.0, 0.0), 0.0);
        assert_eq!(AutomationLane::denormalize(0.5, 1.0, 0.0), 0.5);
    }

    fn lane(points: &[(u64, f64, CurveMode)]) -> AutomationLane {
        AutomationLane {
            id: "test".into(),
            target: AutomationTarget::TrackVolume,
            points: points
                .iter()
                .map(|(t, v, c)| AutomationPoint {
                    tick: *t,
                    value: *v,
                    curve: *c,
                    tension: 0.0,
                })
                .collect(),
            visible: true,
        }
    }

    #[test]
    fn value_at_stays_in_normalized_range_for_normalized_points() {
        let l = lane(&[
            (0, 0.0, CurveMode::Linear),
            (100, 1.0, CurveMode::Linear),
            (200, 0.25, CurveMode::Step),
        ]);
        for tick in 0..=300 {
            let v = l.value_at(tick);
            assert!(
                (0.0..=1.0).contains(&v),
                "value_at({tick}) = {v} escaped [0, 1]"
            );
        }
    }

    #[test]
    fn denormalized_value_at_maps_into_target_range() {
        let l = lane(&[(0, 0.0, CurveMode::Linear), (100, 1.0, CurveMode::Linear)]);
        assert_eq!(l.denormalized_value_at(0, -24.0, 24.0), -24.0);
        assert_eq!(l.denormalized_value_at(100, -24.0, 24.0), 24.0);
        assert_eq!(l.denormalized_value_at(50, -24.0, 24.0), 0.0);
    }

    #[test]
    fn stairs_curve_produces_quantized_steps() {
        // 4-step stairs from 0.0 → 1.0 over 0..100 ticks. Expected values
        // at tick 0, 25, 50, 75 = 0.0, 0.25, 0.5, 0.75; at tick 100
        // we're clamped to b.value == 1.0.
        let l = lane(&[(0, 0.0, CurveMode::Stairs), (100, 1.0, CurveMode::Linear)]);
        assert_eq!(l.value_at(0), 0.0);
        assert_eq!(l.value_at(24), 0.0, "first step holds until boundary");
        assert_eq!(l.value_at(25), 0.25);
        assert_eq!(l.value_at(49), 0.25, "second step holds until boundary");
        assert_eq!(l.value_at(50), 0.5);
        assert_eq!(l.value_at(75), 0.75);
        assert_eq!(l.value_at(100), 1.0);
    }

    #[test]
    fn smooth_stairs_is_monotonic_within_segment() {
        // SmoothStairs still climbs monotonically from 0.0 to 1.0 — no
        // dips, no overshoots. It just replaces the instant jumps of
        // plain stairs with cosine ramps.
        let l = lane(&[
            (0, 0.0, CurveMode::SmoothStairs),
            (400, 1.0, CurveMode::Linear),
        ]);
        let mut prev = -1.0;
        for t in 0..=400 {
            let v = l.value_at(t);
            assert!(
                v >= prev - 1e-9,
                "smooth stairs not monotonic at tick {t}: {v} < prev {prev}"
            );
            assert!((0.0..=1.0).contains(&v));
            prev = v;
        }
    }

    #[test]
    fn smooth_stairs_matches_stairs_at_step_boundaries() {
        // At the start of each of the 4 steps (t = 0, 0.25, 0.5, 0.75
        // of the segment), SmoothStairs and Stairs produce the same
        // value — the smoothing happens *inside* each step, not at the
        // boundaries.
        let stair = lane(&[(0, 0.0, CurveMode::Stairs), (400, 1.0, CurveMode::Linear)]);
        let smooth = lane(&[
            (0, 0.0, CurveMode::SmoothStairs),
            (400, 1.0, CurveMode::Linear),
        ]);
        for t in [0, 100, 200, 300] {
            assert!((stair.value_at(t) - smooth.value_at(t)).abs() < 1e-9);
        }
    }
}
