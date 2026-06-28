//! Automation lane CRUD commands.
//!
//! Each track owns a `Vec<AutomationLane>` (defined in `hardwave-project`).
//! Lanes carry a target (TrackVolume / TrackPan / PluginParam / SendLevel)
//! and a sorted list of points (`tick`, `value`, `curve`). The audio
//! thread re-evaluates lanes per block — see
//! `crates/hardwave-engine/src/track_node.rs`.
//!
//! These commands live UI-side and just shuffle data into the project,
//! then trigger an engine rebuild so the audio thread sees the new
//! lane snapshot. Lock order matches every other tracks command:
//! engine → project; never the other way round.
//!
//! All point lists stay sorted by `tick` to keep `value_at()`'s
//! `partition_point` contract happy.

use crate::AppState;
use hardwave_project::automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveMode};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaneTargetSpec {
    TrackVolume,
    TrackPan,
    TrackMute,
    PluginParam { slot_id: String, param_id: u32 },
    SendLevel { send_index: usize },
}

impl From<LaneTargetSpec> for AutomationTarget {
    fn from(t: LaneTargetSpec) -> Self {
        match t {
            LaneTargetSpec::TrackVolume => AutomationTarget::TrackVolume,
            LaneTargetSpec::TrackPan => AutomationTarget::TrackPan,
            LaneTargetSpec::TrackMute => AutomationTarget::TrackMute,
            LaneTargetSpec::PluginParam { slot_id, param_id } => {
                AutomationTarget::PluginParam { slot_id, param_id }
            }
            LaneTargetSpec::SendLevel { send_index } => AutomationTarget::SendLevel { send_index },
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveSpec {
    Linear,
    Bezier,
    Step,
    Stairs,
    SmoothStairs,
}

impl From<CurveSpec> for CurveMode {
    fn from(c: CurveSpec) -> Self {
        match c {
            CurveSpec::Linear => CurveMode::Linear,
            CurveSpec::Bezier => CurveMode::Bezier,
            CurveSpec::Step => CurveMode::Step,
            CurveSpec::Stairs => CurveMode::Stairs,
            CurveSpec::SmoothStairs => CurveMode::SmoothStairs,
        }
    }
}

/// Append a new automation lane to the track. Returns the freshly-
/// allocated lane id so the UI can address it in subsequent calls.
#[tauri::command]
pub fn add_automation_lane(
    state: State<'_, AppState>,
    track_id: String,
    target: LaneTargetSpec,
) -> Result<String, String> {
    state.engine.lock().snapshot_before_mutation();
    let lane_id = Uuid::new_v4().to_string();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        track.automation_lanes.push(AutomationLane {
            id: lane_id.clone(),
            target: target.into(),
            points: Vec::new(),
            visible: true,
        });
    }
    state.engine.lock().rebuild_graph();
    Ok(lane_id)
}

/// Remove a lane and every point on it. Idempotent — silently no-ops if
/// the lane id isn't found, so a UI race that double-clicks delete
/// can't return an error.
#[tauri::command]
pub fn delete_automation_lane(
    state: State<'_, AppState>,
    track_id: String,
    lane_id: String,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        track.automation_lanes.retain(|l| l.id != lane_id);
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Insert a point keeping the lane's `points` sorted by tick. Returns
/// the index where the point landed so the UI can highlight it
/// without an extra round-trip.
#[tauri::command]
pub fn add_automation_point(
    state: State<'_, AppState>,
    track_id: String,
    lane_id: String,
    tick: u64,
    value: f64,
) -> Result<usize, String> {
    state.engine.lock().snapshot_before_mutation();
    let idx = {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let lane = track
            .automation_lanes
            .iter_mut()
            .find(|l| l.id == lane_id)
            .ok_or_else(|| format!("Lane not found: {lane_id}"))?;
        let point = AutomationPoint {
            tick,
            value: value.clamp(0.0, 1.0),
            curve: CurveMode::Linear,
            tension: 0.0,
        };
        let pos = lane.points.partition_point(|p| p.tick < tick);
        lane.points.insert(pos, point);
        pos
    };
    state.engine.lock().rebuild_graph();
    Ok(idx)
}

/// Drag-move a point. Allows tick AND value to change; resorts the
/// lane afterwards because moving a point past its neighbour breaks
/// the sort invariant. Returns the new index (may differ from the
/// input if the user dragged across another point).
#[tauri::command]
pub fn move_automation_point(
    state: State<'_, AppState>,
    track_id: String,
    lane_id: String,
    point_index: usize,
    tick: u64,
    value: f64,
) -> Result<usize, String> {
    state.engine.lock().snapshot_before_mutation();
    let new_idx = {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let lane = track
            .automation_lanes
            .iter_mut()
            .find(|l| l.id == lane_id)
            .ok_or_else(|| format!("Lane not found: {lane_id}"))?;
        if point_index >= lane.points.len() {
            return Err(format!("Point index out of range: {point_index}"));
        }
        let mut moved = lane.points.remove(point_index);
        moved.tick = tick;
        moved.value = value.clamp(0.0, 1.0);
        let pos = lane.points.partition_point(|p| p.tick < tick);
        lane.points.insert(pos, moved);
        pos
    };
    state.engine.lock().rebuild_graph();
    Ok(new_idx)
}

#[tauri::command]
pub fn delete_automation_point(
    state: State<'_, AppState>,
    track_id: String,
    lane_id: String,
    point_index: usize,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let lane = track
            .automation_lanes
            .iter_mut()
            .find(|l| l.id == lane_id)
            .ok_or_else(|| format!("Lane not found: {lane_id}"))?;
        if point_index < lane.points.len() {
            lane.points.remove(point_index);
        }
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Bulk update for a point's curve mode. The audio thread reads this
/// each block via the lane snapshot, so a change is heard on the next
/// rebuild.
#[tauri::command]
pub fn set_automation_point_curve(
    state: State<'_, AppState>,
    track_id: String,
    lane_id: String,
    point_index: usize,
    curve: CurveSpec,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let lane = track
            .automation_lanes
            .iter_mut()
            .find(|l| l.id == lane_id)
            .ok_or_else(|| format!("Lane not found: {lane_id}"))?;
        if let Some(p) = lane.points.get_mut(point_index) {
            p.curve = curve.into();
        }
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Toggle a lane's visibility. Hidden lanes are skipped during the
/// audio thread's evaluation pass so the user can A/B compare with
/// automation engaged vs bypassed.
#[tauri::command]
pub fn set_automation_lane_visible(
    state: State<'_, AppState>,
    track_id: String,
    lane_id: String,
    visible: bool,
) -> Result<(), String> {
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        if let Some(lane) = track.automation_lanes.iter_mut().find(|l| l.id == lane_id) {
            lane.visible = visible;
        }
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Bake an LFO shape into a lane as automation points spanning
/// `[start_tick, start_tick + length_ticks]`, replacing any existing
/// points inside that window (points outside it are kept). The cycle
/// length is derived from the project's current BPM so tempo-synced
/// rates line up with the grid. The audio thread picks up the new points
/// on the next rebuild — see `track_node.rs`.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn apply_lfo_to_lane(
    state: State<'_, AppState>,
    track_id: String,
    lane_id: String,
    shape: hardwave_project::lfo::LfoShape,
    rate: hardwave_project::lfo::LfoRate,
    depth: f64,
    center: f64,
    phase: f64,
    samples_per_cycle: u32,
    start_tick: u64,
    length_ticks: u64,
) -> Result<usize, String> {
    use std::sync::atomic::Ordering;
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let bpm = engine.transport.bpm.load(Ordering::Relaxed);
    let count = {
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let lane = track
            .automation_lanes
            .iter_mut()
            .find(|l| l.id == lane_id)
            .ok_or_else(|| format!("Lane not found: {lane_id}"))?;
        let generated = hardwave_project::lfo::bake_to_points(
            shape,
            rate,
            bpm,
            hardwave_midi::PPQ,
            start_tick,
            length_ticks,
            depth,
            center,
            phase,
            samples_per_cycle.max(1),
        );
        // Replace points inside the baked window; keep everything outside.
        let end_tick = start_tick + length_ticks;
        lane.points
            .retain(|p| p.tick < start_tick || p.tick > end_tick);
        let count = generated.len();
        lane.points.extend(generated);
        lane.points.sort_by_key(|p| p.tick);
        count
    };
    engine.rebuild_graph();
    Ok(count)
}
