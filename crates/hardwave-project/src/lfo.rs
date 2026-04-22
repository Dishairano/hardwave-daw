//! LFO shape generator — pure math helpers for turning a shape + rate +
//! depth + phase into a stream of automation values. Used by the
//! "Apply LFO shape as automation points" flow: the caller picks a
//! shape/rate/depth/phase, the lane gets pre-baked control points, and
//! `AutomationLane::value_at` takes over from there.

use serde::{Deserialize, Serialize};

use crate::automation::{AutomationPoint, CurveMode};

/// Every LFO shape listed in the roadmap is a variant here. Each one
/// returns a normalized value in `[0.0, 1.0]` for a given phase in
/// `[0.0, 1.0)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LfoShape {
    Sine,
    Triangle,
    Square,
    SawtoothUp,
    SawtoothDown,
    /// Sample-and-hold — value changes once per cycle, held flat between
    /// edges. Deterministic for a given `seed` so playback is repeatable.
    RandomSampleAndHold,
}

/// LFO rate specification. `Hz(f64)` is free-running, `TempoSync(n, d)`
/// is one cycle every `n/d` of a whole note (so `TempoSync(1, 4)` =
/// quarter-note cycle).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LfoRate {
    Hz(f64),
    TempoSync { num: u32, den: u32 },
}

impl LfoRate {
    /// Cycle length in ticks, given the tempo and ticks-per-quarter.
    pub fn cycle_length_ticks(&self, bpm: f64, ppq: u64) -> u64 {
        match self {
            LfoRate::Hz(hz) if *hz > 0.0 => {
                // ticks / cycle = (ppq * bpm / 60) / hz
                let ticks_per_second = ppq as f64 * bpm / 60.0;
                (ticks_per_second / hz).max(1.0) as u64
            }
            LfoRate::Hz(_) => u64::MAX / 2,
            LfoRate::TempoSync { num, den } if *den > 0 && *num > 0 => {
                // One whole note = 4 quarter-notes = 4 * ppq ticks.
                let whole_note = 4 * ppq;
                whole_note * (*num as u64) / (*den as u64)
            }
            LfoRate::TempoSync { .. } => u64::MAX / 2,
        }
    }
}

/// Evaluate a shape at a given phase `[0.0, 1.0)`, returning a value in
/// `[0.0, 1.0]`. `seed` only matters for `RandomSampleAndHold`.
pub fn sample_shape(shape: LfoShape, phase: f64, seed: u64) -> f64 {
    let p = phase.rem_euclid(1.0);
    match shape {
        LfoShape::Sine => 0.5 + 0.5 * (2.0 * std::f64::consts::PI * p).sin(),
        LfoShape::Triangle => {
            if p < 0.5 {
                p * 2.0
            } else {
                2.0 - p * 2.0
            }
        }
        LfoShape::Square => {
            if p < 0.5 {
                1.0
            } else {
                0.0
            }
        }
        LfoShape::SawtoothUp => p,
        LfoShape::SawtoothDown => 1.0 - p,
        LfoShape::RandomSampleAndHold => {
            // xorshift-based deterministic noise keyed on (seed, cycle).
            // `phase` is already mod 1, so one value per cycle.
            let cycle_index = (p * 16.0).floor() as u64;
            let mut x = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(cycle_index);
            x ^= x >> 33;
            x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
            x ^= x >> 33;
            x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
            x ^= x >> 33;
            (x as f64) / (u64::MAX as f64)
        }
    }
}

/// Bake an LFO shape as a set of automation points spanning
/// `[start_tick, start_tick + length_ticks]`. `depth` scales the
/// shape's 0..1 range around `center`; `phase_offset` is in `[0, 1)`
/// and shifts the waveform. `samples_per_cycle` controls how densely
/// the waveform is sampled — 32 is usually enough for a smooth sine.
#[allow(clippy::too_many_arguments)]
pub fn bake_to_points(
    shape: LfoShape,
    rate: LfoRate,
    bpm: f64,
    ppq: u64,
    start_tick: u64,
    length_ticks: u64,
    depth: f64,
    center: f64,
    phase_offset: f64,
    samples_per_cycle: u32,
) -> Vec<AutomationPoint> {
    if length_ticks == 0 || samples_per_cycle == 0 {
        return Vec::new();
    }
    let cycle = rate.cycle_length_ticks(bpm, ppq).max(1);
    let sample_stride = (cycle / samples_per_cycle as u64).max(1);
    let seed = 0xDEADBEEF_u64;
    let mut points = Vec::new();
    let mut tick = start_tick;
    let end_tick = start_tick + length_ticks;
    while tick <= end_tick {
        let local = tick - start_tick;
        let phase = (local as f64 / cycle as f64 + phase_offset).rem_euclid(1.0);
        let shape_val = sample_shape(shape, phase, seed);
        let centered = (shape_val - 0.5) * depth.clamp(0.0, 1.0);
        let value = (center + centered).clamp(0.0, 1.0);
        let curve = match shape {
            LfoShape::Square | LfoShape::RandomSampleAndHold => CurveMode::Step,
            _ => CurveMode::Linear,
        };
        points.push(AutomationPoint {
            tick,
            value,
            curve,
            tension: 0.0,
        });
        tick = tick.saturating_add(sample_stride);
        if tick == start_tick {
            break; // paranoia against wrap
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_hits_midpoint_and_peaks() {
        assert!((sample_shape(LfoShape::Sine, 0.0, 0) - 0.5).abs() < 1e-9);
        assert!((sample_shape(LfoShape::Sine, 0.25, 0) - 1.0).abs() < 1e-9);
        assert!((sample_shape(LfoShape::Sine, 0.5, 0) - 0.5).abs() < 1e-9);
        assert!((sample_shape(LfoShape::Sine, 0.75, 0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn triangle_linear_up_then_down() {
        assert_eq!(sample_shape(LfoShape::Triangle, 0.0, 0), 0.0);
        assert_eq!(sample_shape(LfoShape::Triangle, 0.25, 0), 0.5);
        assert_eq!(sample_shape(LfoShape::Triangle, 0.5, 0), 1.0);
        assert_eq!(sample_shape(LfoShape::Triangle, 0.75, 0), 0.5);
    }

    #[test]
    fn square_holds_high_then_low() {
        assert_eq!(sample_shape(LfoShape::Square, 0.0, 0), 1.0);
        assert_eq!(sample_shape(LfoShape::Square, 0.49, 0), 1.0);
        assert_eq!(sample_shape(LfoShape::Square, 0.5, 0), 0.0);
        assert_eq!(sample_shape(LfoShape::Square, 0.99, 0), 0.0);
    }

    #[test]
    fn sawtooth_up_ramps_0_to_1() {
        assert_eq!(sample_shape(LfoShape::SawtoothUp, 0.0, 0), 0.0);
        assert!((sample_shape(LfoShape::SawtoothUp, 0.5, 0) - 0.5).abs() < 1e-9);
        assert!(sample_shape(LfoShape::SawtoothUp, 0.999, 0) > 0.99);
    }

    #[test]
    fn sawtooth_down_ramps_1_to_0() {
        assert_eq!(sample_shape(LfoShape::SawtoothDown, 0.0, 0), 1.0);
        assert!((sample_shape(LfoShape::SawtoothDown, 0.5, 0) - 0.5).abs() < 1e-9);
        assert!(sample_shape(LfoShape::SawtoothDown, 0.999, 0) < 0.01);
    }

    #[test]
    fn sample_and_hold_is_deterministic() {
        let a = sample_shape(LfoShape::RandomSampleAndHold, 0.1, 42);
        let b = sample_shape(LfoShape::RandomSampleAndHold, 0.1, 42);
        assert_eq!(a, b, "same seed + phase must produce same value");
        assert!((0.0..=1.0).contains(&a));
    }

    #[test]
    fn hz_rate_maps_to_ticks_per_second() {
        // At 120 BPM, 1 Hz = 1 cycle per second = 2 beats = 2 * 960 ticks.
        let rate = LfoRate::Hz(1.0);
        assert_eq!(rate.cycle_length_ticks(120.0, 960), 1920);
    }

    #[test]
    fn tempo_sync_rate_quarter_note() {
        // TempoSync(1, 4) = quarter note = 1 beat = 960 ticks at PPQ 960.
        let rate = LfoRate::TempoSync { num: 1, den: 4 };
        assert_eq!(rate.cycle_length_ticks(120.0, 960), 960);
    }

    #[test]
    fn tempo_sync_rate_one_bar_equals_4_beats() {
        // TempoSync(1, 1) = one whole note = 4 beats = 4 * 960 ticks.
        let rate = LfoRate::TempoSync { num: 1, den: 1 };
        assert_eq!(rate.cycle_length_ticks(120.0, 960), 3840);
    }

    #[test]
    fn bake_produces_points_across_full_length() {
        let points = bake_to_points(
            LfoShape::Sine,
            LfoRate::TempoSync { num: 1, den: 4 },
            120.0,
            960,
            0,
            3840,
            1.0,
            0.5,
            0.0,
            32,
        );
        assert!(points.len() >= 32);
        assert_eq!(points.first().unwrap().tick, 0);
        assert!(points.last().unwrap().tick <= 3840);
        for p in &points {
            assert!((0.0..=1.0).contains(&p.value));
        }
    }

    #[test]
    fn phase_offset_shifts_output() {
        // Sine at phase=0 → 0.5; phase=0.25 → 1.0. Offsetting baking
        // by 0.25 should push the first point's value up toward 1.0.
        let pts_offset = bake_to_points(
            LfoShape::Sine,
            LfoRate::TempoSync { num: 1, den: 4 },
            120.0,
            960,
            0,
            960,
            1.0,
            0.5,
            0.25,
            32,
        );
        assert!(pts_offset.first().unwrap().value > 0.95);
    }

    #[test]
    fn depth_zero_produces_flat_center_line() {
        let pts = bake_to_points(
            LfoShape::Triangle,
            LfoRate::Hz(1.0),
            120.0,
            960,
            0,
            1920,
            0.0,
            0.5,
            0.0,
            16,
        );
        for p in &pts {
            assert!((p.value - 0.5).abs() < 1e-9);
        }
    }
}
