//! Automation clips — arrangement-level clips that carry an
//! `AutomationLane` targeting a specific parameter, with a start /
//! length in ticks and layering semantics for multiple clips on the
//! same parameter.

use crate::automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveMode};
use serde::{Deserialize, Serialize};

/// One automation clip placed on a track. The clip's lane stores
/// points in `clip-local` tick space; `timeline_tick_to_clip(t)`
/// converts the playhead's timeline tick into the clip's coordinate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationClip {
    pub id: String,
    pub target: AutomationTarget,
    pub start_tick: u64,
    pub length_ticks: u64,
    pub color_argb: u32,
    pub lane: AutomationLane,
}

/// Options returned by a right-click on an automation point — the UI
/// renders a menu from this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointAction {
    ChangeCurveLinear,
    ChangeCurveBezier,
    ChangeCurveStep,
    ChangeCurveStairs,
    ChangeCurveSmoothStairs,
    DeletePoint,
    ResetValue,
}

impl AutomationClip {
    pub fn new(
        id: impl Into<String>,
        target: AutomationTarget,
        start_tick: u64,
        length_ticks: u64,
    ) -> Self {
        let target_clone = target.clone();
        Self {
            id: id.into(),
            target,
            start_tick,
            length_ticks,
            color_argb: color_for_target(&target_clone),
            lane: AutomationLane {
                id: format!("{}-lane", 0),
                target: target_clone,
                points: Vec::new(),
                visible: true,
            },
        }
    }

    /// Add a control point. Ticks outside `[0, length_ticks]` are
    /// clamped to the clip bounds. Points are kept sorted by tick.
    /// Adjacent duplicates (same tick) overwrite existing values.
    pub fn insert_point(&mut self, tick: u64, value: f64, curve: CurveMode) {
        let tick = tick.min(self.length_ticks);
        let value = value.clamp(0.0, 1.0);
        let new_point = AutomationPoint {
            tick,
            value,
            curve,
            tension: 0.0,
        };
        match self.lane.points.binary_search_by_key(&tick, |p| p.tick) {
            Ok(existing) => self.lane.points[existing] = new_point,
            Err(pos) => self.lane.points.insert(pos, new_point),
        }
    }

    /// Remove the point at index `idx`. No-op if out of range.
    pub fn remove_point(&mut self, idx: usize) -> Option<AutomationPoint> {
        if idx < self.lane.points.len() {
            Some(self.lane.points.remove(idx))
        } else {
            None
        }
    }

    /// Drag a point: change tick + value. Clamps to clip bounds and
    /// re-sorts if needed.
    pub fn move_point(&mut self, idx: usize, new_tick: u64, new_value: f64) -> bool {
        if idx >= self.lane.points.len() {
            return false;
        }
        let tick = new_tick.min(self.length_ticks);
        let value = new_value.clamp(0.0, 1.0);
        self.lane.points[idx].tick = tick;
        self.lane.points[idx].value = value;
        // Re-sort — the move might have crossed a neighbor.
        self.lane.points.sort_by_key(|p| p.tick);
        true
    }

    /// Convert a timeline tick to a clip-local tick. Returns `None`
    /// if the playhead is outside the clip window.
    pub fn timeline_tick_to_clip(&self, timeline_tick: u64) -> Option<u64> {
        if timeline_tick < self.start_tick {
            return None;
        }
        let rel = timeline_tick - self.start_tick;
        if rel >= self.length_ticks {
            return None;
        }
        Some(rel)
    }

    /// Sample the clip's value at a timeline tick. `None` if the
    /// clip doesn't contain the tick. Value is in `[0, 1]`.
    pub fn value_at_timeline(&self, timeline_tick: u64) -> Option<f64> {
        let local = self.timeline_tick_to_clip(timeline_tick)?;
        Some(self.lane.value_at(local))
    }

    /// Apply a `PointAction` to the point at `idx`. Returns whether
    /// the action modified state.
    pub fn apply_point_action(&mut self, idx: usize, action: PointAction) -> bool {
        if idx >= self.lane.points.len() {
            return false;
        }
        match action {
            PointAction::ChangeCurveLinear => self.lane.points[idx].curve = CurveMode::Linear,
            PointAction::ChangeCurveBezier => self.lane.points[idx].curve = CurveMode::Bezier,
            PointAction::ChangeCurveStep => self.lane.points[idx].curve = CurveMode::Step,
            PointAction::ChangeCurveStairs => self.lane.points[idx].curve = CurveMode::Stairs,
            PointAction::ChangeCurveSmoothStairs => {
                self.lane.points[idx].curve = CurveMode::SmoothStairs
            }
            PointAction::DeletePoint => {
                self.lane.points.remove(idx);
            }
            PointAction::ResetValue => self.lane.points[idx].value = 0.5,
        }
        true
    }

    /// Render the clip's filled-curve visual — returns `samples`
    /// values evenly spaced across the clip's length. Useful for the
    /// arrangement view's filled-area rendering.
    pub fn render_curve(&self, samples: usize) -> Vec<f64> {
        if samples == 0 || self.length_ticks == 0 {
            return Vec::new();
        }
        (0..samples)
            .map(|i| {
                let tick =
                    (i as u64).saturating_mul(self.length_ticks - 1) / (samples as u64 - 1).max(1);
                self.lane.value_at(tick)
            })
            .collect()
    }

    /// Preview-tooltip value: return the value at the nearest
    /// hoverable tick, clamped to clip bounds.
    pub fn preview_value_at(&self, timeline_tick: u64) -> f64 {
        let local = timeline_tick
            .saturating_sub(self.start_tick)
            .min(self.length_ticks);
        self.lane.value_at(local)
    }
}

/// Sample multiple automation clips targeting the same parameter at
/// the given timeline tick. Layering is "last-writer-wins" — the
/// clip that actually contains the tick and starts latest wins. If
/// no clip contains the tick, returns `None` so the caller can fall
/// back to the parameter's static value.
pub fn sample_layered<'a, I>(clips: I, timeline_tick: u64) -> Option<f64>
where
    I: IntoIterator<Item = &'a AutomationClip>,
{
    let mut winner: Option<&AutomationClip> = None;
    for c in clips {
        if c.timeline_tick_to_clip(timeline_tick).is_some() {
            match winner {
                None => winner = Some(c),
                Some(w) if c.start_tick > w.start_tick => winner = Some(c),
                _ => {}
            }
        }
    }
    winner?.value_at_timeline(timeline_tick)
}

fn color_for_target(target: &AutomationTarget) -> u32 {
    // Stable hash → saturated ARGB. Different targets always get
    // visually distinct colors so the UI doesn't need a palette table.
    let mut hash: u32 = 0x811C9DC5;
    let key = format!("{:?}", target);
    for byte in key.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    // Clamp saturation: force high brightness + some alpha.
    let r = ((hash >> 16) & 0xFF) | 0x40;
    let g = ((hash >> 8) & 0xFF) | 0x40;
    let b = (hash & 0xFF) | 0x40;
    0xFF_00_00_00 | (r << 16) | (g << 8) | b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_clip() -> AutomationClip {
        AutomationClip::new("c1", AutomationTarget::TrackVolume, 100, 400)
    }

    #[test]
    fn insert_point_is_clamped_and_sorted() {
        let mut c = mk_clip();
        c.insert_point(200, 0.5, CurveMode::Linear);
        c.insert_point(100, 0.2, CurveMode::Linear);
        c.insert_point(1_000, 1.5, CurveMode::Linear); // clamps tick + value
        let ticks: Vec<u64> = c.lane.points.iter().map(|p| p.tick).collect();
        let values: Vec<f64> = c.lane.points.iter().map(|p| p.value).collect();
        assert_eq!(ticks, vec![100, 200, 400]);
        assert_eq!(values, vec![0.2, 0.5, 1.0]);
    }

    #[test]
    fn insert_point_overwrites_duplicate_tick() {
        let mut c = mk_clip();
        c.insert_point(100, 0.2, CurveMode::Linear);
        c.insert_point(100, 0.8, CurveMode::Linear);
        assert_eq!(c.lane.points.len(), 1);
        assert!((c.lane.points[0].value - 0.8).abs() < 1e-9);
    }

    #[test]
    fn move_point_resorts_and_clamps() {
        let mut c = mk_clip();
        c.insert_point(50, 0.1, CurveMode::Linear);
        c.insert_point(150, 0.9, CurveMode::Linear);
        assert!(c.move_point(0, 500, 2.0));
        let ticks: Vec<u64> = c.lane.points.iter().map(|p| p.tick).collect();
        assert_eq!(ticks, vec![150, 400]);
        let moved = &c.lane.points[1];
        assert!((moved.value - 1.0).abs() < 1e-9);
    }

    #[test]
    fn remove_point_and_bounds() {
        let mut c = mk_clip();
        c.insert_point(50, 0.1, CurveMode::Linear);
        c.insert_point(150, 0.9, CurveMode::Linear);
        assert!(c.remove_point(0).is_some());
        assert!(c.remove_point(10).is_none());
        assert_eq!(c.lane.points.len(), 1);
    }

    #[test]
    fn timeline_to_clip_outside_window_is_none() {
        let c = mk_clip();
        assert!(c.timeline_tick_to_clip(50).is_none());
        assert!(c.timeline_tick_to_clip(500).is_none());
        assert_eq!(c.timeline_tick_to_clip(200), Some(100));
    }

    #[test]
    fn value_at_timeline_samples_correctly() {
        let mut c = mk_clip();
        c.insert_point(0, 0.0, CurveMode::Linear);
        c.insert_point(400, 1.0, CurveMode::Linear);
        assert_eq!(c.value_at_timeline(100), Some(0.0));
        assert_eq!(c.value_at_timeline(300), Some(0.5));
        // Clip spans [100, 500): tick 499 is the last sample, 500 is OOB.
        let near_end = c.value_at_timeline(499).unwrap();
        assert!(near_end > 0.99);
        assert!(c.value_at_timeline(500).is_none());
        assert!(c.value_at_timeline(50).is_none());
    }

    #[test]
    fn render_curve_returns_n_samples_across_clip() {
        let mut c = mk_clip();
        c.insert_point(0, 0.0, CurveMode::Linear);
        c.insert_point(400, 1.0, CurveMode::Linear);
        let v = c.render_curve(5);
        assert_eq!(v.len(), 5);
        assert!(v[0].abs() < 1e-9);
        // render_curve samples at tick indices 0..length_ticks-1, so
        // the last sample falls just short of 1.0 but close.
        assert!(v[4] > 0.99);
    }

    #[test]
    fn point_action_changes_curve_and_deletes() {
        let mut c = mk_clip();
        c.insert_point(0, 0.3, CurveMode::Linear);
        c.insert_point(100, 0.8, CurveMode::Linear);
        assert!(c.apply_point_action(0, PointAction::ChangeCurveStep));
        assert!(matches!(c.lane.points[0].curve, CurveMode::Step));
        assert!(c.apply_point_action(0, PointAction::DeletePoint));
        assert_eq!(c.lane.points.len(), 1);
    }

    #[test]
    fn layered_clips_resolve_by_latest_start() {
        let mut a = AutomationClip::new("a", AutomationTarget::TrackVolume, 0, 1_000);
        a.insert_point(0, 0.0, CurveMode::Linear);
        a.insert_point(1_000, 0.2, CurveMode::Linear);
        let mut b = AutomationClip::new("b", AutomationTarget::TrackVolume, 200, 500);
        b.insert_point(0, 0.9, CurveMode::Linear);
        b.insert_point(500, 0.9, CurveMode::Linear);
        let clips = [&a, &b];
        // Tick 100 — only `a` covers → returns a's value.
        let v100 = sample_layered(clips.iter().copied(), 100);
        assert!(v100.unwrap() < 0.3);
        // Tick 400 — both cover; `b` starts later → wins with 0.9.
        let v400 = sample_layered(clips.iter().copied(), 400).unwrap();
        assert!((v400 - 0.9).abs() < 1e-9);
        // Tick 2000 — neither covers.
        assert!(sample_layered(clips.iter().copied(), 2_000).is_none());
    }

    #[test]
    fn clip_color_is_stable_and_distinct() {
        let c1 = AutomationClip::new("x", AutomationTarget::TrackVolume, 0, 100);
        let c2 = AutomationClip::new("x", AutomationTarget::TrackVolume, 0, 100);
        assert_eq!(c1.color_argb, c2.color_argb);
        let c3 = AutomationClip::new("x", AutomationTarget::TrackPan, 0, 100);
        assert_ne!(c1.color_argb, c3.color_argb);
    }
}
