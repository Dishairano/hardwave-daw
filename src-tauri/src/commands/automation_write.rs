//! Recording a control movement into an automation lane.
//!
//! `hardwave_project::automation_recording::AutomationRecorder` has existed,
//! with tests, since the automation work: it turns a stream of
//! `(tick, value)` samples into `AutomationPoint`s, with the classic Off,
//! Read, Write, Touch and Latch modes. Nothing called it. `push_sample` had
//! no caller outside the engine's own test, so moving a fader during playback
//! wrote no automation at all and the feature existed only on paper.
//!
//! These commands are that missing caller. The UI says when a control is
//! touched, streams its values while the transport runs, and says when it is
//! let go; the session here stamps each value with the playhead in ticks,
//! through the project's tempo map, and bakes the result into the target's
//! lane.

use crate::commands::automation::LaneTargetSpec;
use crate::AppState;
use hardwave_project::automation::{AutomationLane, AutomationTarget, CurveMode};
use hardwave_project::automation_recording::{AutomationRecorder, WriteMode};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;

/// Live recording sessions, keyed by track and target.
///
/// One control can be recorded at a time per target, but several controls can
/// be recorded at once, which is what a two-handed filter and volume move is.
pub struct AutomationWriteSessions {
    sessions: Mutex<HashMap<(String, String), AutomationRecorder>>,
    mode: Mutex<WriteMode>,
}

impl Default for AutomationWriteSessions {
    fn default() -> Self {
        Self::new()
    }
}

impl AutomationWriteSessions {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            mode: Mutex::new(WriteMode::Off),
        }
    }
}

/// A stable key for a target, so two parameters of the same plug-in do not
/// share a session.
fn target_key(target: &AutomationTarget) -> String {
    match target {
        AutomationTarget::TrackVolume => "volume".to_string(),
        AutomationTarget::TrackPan => "pan".to_string(),
        AutomationTarget::TrackMute => "mute".to_string(),
        AutomationTarget::PluginParam { slot_id, param_id } => {
            format!("param:{slot_id}:{param_id}")
        }
        AutomationTarget::SendLevel { send_index } => format!("send:{send_index}"),
    }
}

fn parse_mode(mode: &str) -> WriteMode {
    match mode {
        "read" => WriteMode::Read,
        "write" => WriteMode::Write,
        "touch" => WriteMode::Touch,
        "latch" => WriteMode::Latch,
        _ => WriteMode::Off,
    }
}

/// Set the automation write mode for the session.
#[tauri::command]
pub fn set_automation_write_mode(state: State<AppState>, mode: String) {
    let parsed = parse_mode(&mode);
    *state.automation_write.mode.lock().unwrap() = parsed;
    // Leaving write behind drops anything half-recorded rather than baking a
    // partial move when the mode changes mid-pass.
    if matches!(parsed, WriteMode::Off | WriteMode::Read) {
        state.automation_write.sessions.lock().unwrap().clear();
    }
}

#[tauri::command]
pub fn get_automation_write_mode(state: State<AppState>) -> String {
    match *state.automation_write.mode.lock().unwrap() {
        WriteMode::Off => "off",
        WriteMode::Read => "read",
        WriteMode::Write => "write",
        WriteMode::Touch => "touch",
        WriteMode::Latch => "latch",
    }
    .to_string()
}

/// A control was touched. Starts a session when the mode records.
#[tauri::command]
pub fn automation_touch_begin(state: State<AppState>, track_id: String, target: LaneTargetSpec) {
    let mode = *state.automation_write.mode.lock().unwrap();
    if matches!(mode, WriteMode::Off | WriteMode::Read) {
        return;
    }
    let target: AutomationTarget = target.into();
    let mut recorder = AutomationRecorder::default();
    recorder.set_mode(mode);
    // Write mode captures from the moment the transport rolls; touch and
    // latch capture from the touch itself.
    recorder.on_transport_play();
    recorder.begin_touch();
    state
        .automation_write
        .sessions
        .lock()
        .unwrap()
        .insert((track_id, target_key(&target)), recorder);
}

/// One value from a control being moved. Ignored when nothing is recording.
#[tauri::command]
pub fn automation_write_sample(
    state: State<AppState>,
    track_id: String,
    target: LaneTargetSpec,
    value: f64,
) {
    let target: AutomationTarget = target.into();
    let key = (track_id, target_key(&target));
    let tick = {
        let engine = state.engine.lock();
        let sample_rate = engine.current_sample_rate() as f64;
        let position = engine.transport.position();
        let project = engine.project.lock();
        project.tempo_map.samples_to_tick(position, sample_rate)
    };
    let mut sessions = state.automation_write.sessions.lock().unwrap();
    if let Some(recorder) = sessions.get_mut(&key) {
        if recorder.is_recording() {
            recorder.push_sample(tick, value.clamp(0.0, 1.0));
        }
    }
}

/// The control was let go. Bakes what was captured into the lane.
///
/// Returns the lane id when something was written, so the UI can show the
/// lane it just filled.
#[tauri::command]
pub fn automation_touch_end(
    state: State<AppState>,
    track_id: String,
    target: LaneTargetSpec,
) -> Option<String> {
    let target: AutomationTarget = target.into();
    let key = (track_id.clone(), target_key(&target));
    let mut recorder = state
        .automation_write
        .sessions
        .lock()
        .unwrap()
        .remove(&key)?;
    recorder.end_touch();
    // A pass with one sample is a click, not a move, and would leave a lane
    // holding a single point that silently overrides the static value.
    if recorder.sample_count() < 2 {
        return None;
    }
    // Thin the stream before baking: a mouse drag produces a sample per
    // frame, and a lane with hundreds of points per second is unreadable and
    // slow to draw for no added accuracy.
    recorder.thin(0.005);
    let points = recorder.into_points(CurveMode::Linear);
    if points.is_empty() {
        return None;
    }

    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    let lane_id = {
        let mut project = engine.project.lock();
        let track = project.track_mut(&track_id)?;
        // Into the lane that already targets this parameter, if there is one,
        // so a second pass replaces the first rather than stacking lanes.
        match track
            .automation_lanes
            .iter_mut()
            .find(|l| automation_targets_match(&l.target, &target))
        {
            Some(lane) => {
                replace_range(&mut lane.points, points);
                lane.visible = true;
                lane.id.clone()
            }
            None => {
                let lane = AutomationLane {
                    id: uuid::Uuid::new_v4().to_string(),
                    target,
                    points,
                    visible: true,
                };
                let id = lane.id.clone();
                track.automation_lanes.push(lane);
                id
            }
        }
    };
    engine.rebuild_graph();
    Some(lane_id)
}

/// Whether two targets are the same parameter.
fn automation_targets_match(a: &AutomationTarget, b: &AutomationTarget) -> bool {
    target_key(a) == target_key(b)
}

/// Put the recorded points in, dropping the ones they cover.
///
/// A recording pass owns the range it covered: leaving the old points in
/// place would fight the new ones for the same ticks.
fn replace_range(
    existing: &mut Vec<hardwave_project::automation::AutomationPoint>,
    recorded: Vec<hardwave_project::automation::AutomationPoint>,
) {
    let (from, to) = match (recorded.first(), recorded.last()) {
        (Some(f), Some(l)) => (f.tick, l.tick),
        _ => return,
    };
    existing.retain(|p| p.tick < from || p.tick > to);
    existing.extend(recorded);
    existing.sort_by_key(|p| p.tick);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_parameters_of_one_plugin_are_separate_sessions() {
        let a = AutomationTarget::PluginParam {
            slot_id: "s1".into(),
            param_id: 7,
        };
        let b = AutomationTarget::PluginParam {
            slot_id: "s1".into(),
            param_id: 8,
        };
        assert_ne!(target_key(&a), target_key(&b));
        assert!(automation_targets_match(&a, &a.clone()));
        assert!(!automation_targets_match(&a, &b));
    }

    #[test]
    fn a_recorded_pass_replaces_what_it_covered() {
        use hardwave_project::automation::AutomationPoint;
        let point = |tick: u64, value: f64| AutomationPoint {
            tick,
            value,
            curve: CurveMode::Linear,
            tension: 0.0,
        };
        let mut existing = vec![point(0, 0.1), point(500, 0.2), point(1_500, 0.3)];
        replace_range(&mut existing, vec![point(400, 0.9), point(1_000, 0.8)]);

        let ticks: Vec<u64> = existing.iter().map(|p| p.tick).collect();
        assert_eq!(
            ticks,
            vec![0, 400, 1_000, 1_500],
            "the covered point stayed"
        );
    }

    #[test]
    fn a_pass_that_covers_nothing_keeps_every_point() {
        use hardwave_project::automation::AutomationPoint;
        let point = |tick: u64| AutomationPoint {
            tick,
            value: 0.5,
            curve: CurveMode::Linear,
            tension: 0.0,
        };
        let mut existing = vec![point(0), point(100)];
        replace_range(&mut existing, Vec::new());
        assert_eq!(existing.len(), 2);
    }

    #[test]
    fn modes_parse_from_what_the_ui_sends() {
        assert_eq!(parse_mode("write"), WriteMode::Write);
        assert_eq!(parse_mode("touch"), WriteMode::Touch);
        assert_eq!(parse_mode("latch"), WriteMode::Latch);
        assert_eq!(parse_mode("read"), WriteMode::Read);
        assert_eq!(parse_mode("off"), WriteMode::Off);
        assert_eq!(parse_mode("nonsense"), WriteMode::Off);
    }
}
