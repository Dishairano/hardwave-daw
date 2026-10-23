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
    sessions: Mutex<HashMap<(String, String), Pass>>,
    mode: Mutex<WriteMode>,
    /// Trim: a pass rides the curve that is already there instead of
    /// replacing it. Recording a fader move over an existing ride used to
    /// throw that ride away, so a chorus that was already automated to duck
    /// under the vocal came back flat.
    trim: Mutex<bool>,
    /// Where playback last started, in ticks: a Write pass starts there.
    play_start_tick: Mutex<Option<u64>>,
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
            trim: Mutex::new(false),
            play_start_tick: Mutex::new(None),
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

/// Turn trim on or off. It applies to whichever write mode is selected.
#[tauri::command]
pub fn set_automation_trim(state: State<AppState>, trim: bool) {
    *state.automation_write.trim.lock().unwrap() = trim;
}

#[tauri::command]
pub fn get_automation_trim(state: State<AppState>) -> bool {
    *state.automation_write.trim.lock().unwrap()
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

/// The playhead, in ticks through the song's tempo map.
fn playhead_tick(engine: &hardwave_engine::DawEngine) -> u64 {
    let sample_rate = engine.current_sample_rate() as f64;
    let position = engine.transport.position();
    let project = engine.project.lock();
    project.tempo_map.samples_to_tick(position, sample_rate)
}

/// Playback started. A Write pass covers the song from here, so a control
/// touched later is written from this point on, at the value it had.
pub fn note_play_start(state: &AppState, engine: &hardwave_engine::DawEngine) {
    *state.automation_write.play_start_tick.lock().unwrap() = Some(playhead_tick(engine));
}

/// Playback stopped. Latch and Write passes run until here, holding the
/// last value after the control was let go, and are written into their
/// lanes now; Touch passes were written when their controls were let go.
/// Takes the engine its caller already holds.
pub fn finish_passes(state: &AppState, engine: &hardwave_engine::DawEngine) {
    *state.automation_write.play_start_tick.lock().unwrap() = None;
    let open: Vec<((String, String), Pass)> = state
        .automation_write
        .sessions
        .lock()
        .unwrap()
        .drain()
        .collect();
    if open.is_empty() {
        return;
    }
    let stop_tick = playhead_tick(engine);
    let trim = *state.automation_write.trim.lock().unwrap();
    let mut changed = false;
    for ((track_id, _), mut pass) in open {
        pass.hold_until(stop_tick);
        changed |= bake_into_lane(engine, &track_id, pass, trim).is_some();
    }
    if changed {
        engine.rebuild_graph();
    }
}

/// A control was touched. Starts a pass when the mode records, or picks up
/// the one already running for it (Latch and Write keep theirs after a
/// release, until the song stops).
#[tauri::command]
pub fn automation_touch_begin(state: State<AppState>, track_id: String, target: LaneTargetSpec) {
    let mode = *state.automation_write.mode.lock().unwrap();
    if matches!(mode, WriteMode::Off | WriteMode::Read) {
        return;
    }
    let target: AutomationTarget = target.into();
    let mut sessions = state.automation_write.sessions.lock().unwrap();
    let pass = sessions
        .entry((track_id, target_key(&target)))
        .or_insert_with(|| {
            let mut recorder = AutomationRecorder::default();
            recorder.set_mode(mode);
            recorder.on_transport_play();
            Pass {
                target,
                recorder,
                from_play_start: matches!(mode, WriteMode::Write),
            }
        });
    pass.recorder.begin_touch();
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
    let tick = playhead_tick(&state.engine.lock());
    let play_start = *state.automation_write.play_start_tick.lock().unwrap();
    let value = value.clamp(0.0, 1.0);
    let mut sessions = state.automation_write.sessions.lock().unwrap();
    if let Some(pass) = sessions.get_mut(&key) {
        if !pass.recorder.is_recording() {
            return;
        }
        // Write covers the song from where playback started: the stretch
        // before the first move holds the value the control had.
        if pass.from_play_start && pass.recorder.sample_count() == 0 {
            if let Some(start) = play_start.filter(|&t| t < tick) {
                pass.recorder.push_sample(start, value);
            }
        }
        pass.recorder.push_sample(tick, value);
    }
}

/// The control was let go. A Touch pass is written into the lane now; Latch
/// and Write keep going, holding the last value, until the song stops.
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
    let pass = {
        let mut sessions = state.automation_write.sessions.lock().unwrap();
        if !sessions.get_mut(&key)?.release() {
            return None;
        }
        sessions.remove(&key)?
    };
    let trim = *state.automation_write.trim.lock().unwrap();
    let engine = state.engine.lock();
    let lane = bake_into_lane(&engine, &track_id, pass, trim);
    if lane.is_some() {
        engine.rebuild_graph();
    }
    lane
}

/// One control being recorded: what it drives, and what was captured.
struct Pass {
    target: AutomationTarget,
    recorder: AutomationRecorder,
    /// A Write pass, which starts where playback started.
    from_play_start: bool,
}

impl Pass {
    /// The control was let go. True when the pass ends here (Touch); Latch
    /// and Write go on to the stop.
    fn release(&mut self) -> bool {
        self.recorder.end_touch();
        matches!(self.recorder.mode(), WriteMode::Touch)
    }

    /// The song stopped at `tick`: the last value holds until there.
    fn hold_until(&mut self, tick: u64) {
        if let Some(last) = self.recorder.last_value() {
            self.recorder.push_sample(tick, last);
        }
    }
}

/// Put a finished pass into its lane. The caller rebuilds the graph.
fn bake_into_lane(
    engine: &hardwave_engine::DawEngine,
    track_id: &str,
    pass: Pass,
    trim: bool,
) -> Option<String> {
    let Pass {
        target,
        mut recorder,
        ..
    } = pass;
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

    engine.snapshot_before_mutation();
    let mut project = engine.project.lock();
    let track = project.track_mut(track_id)?;
    if trim {
        // Ride what is there: every point in the range moves by how far
        // the fader moved from where the pass started, so the shape of
        // the existing ride is kept and only its level changes.
        let lane = track
            .automation_lanes
            .iter_mut()
            .find(|l| automation_targets_match(&l.target, &target))?;
        let from = points.first().map(|p| p.tick).unwrap_or(0);
        let to = points.last().map(|p| p.tick).unwrap_or(0);
        let base = points.first().map(|p| p.value).unwrap_or(0.0);
        for point in lane.points.iter_mut() {
            if point.tick < from || point.tick > to {
                continue;
            }
            let delta = value_at(&points, point.tick) - base;
            point.value = (point.value + delta).clamp(0.0, 1.0);
        }
        lane.visible = true;
        return Some(lane.id.clone());
    }
    // Into the lane that already targets this parameter, if there is one,
    // so a second pass replaces the first rather than stacking lanes.
    let id = match track
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
    };
    Some(id)
}

/// The recorded pass's value at a tick, walking the points it produced.
/// Straight lines between points, which is what the pass was thinned to.
fn value_at(points: &[hardwave_project::automation::AutomationPoint], tick: u64) -> f64 {
    if points.is_empty() {
        return 0.0;
    }
    if tick <= points[0].tick {
        return points[0].value;
    }
    let last = &points[points.len() - 1];
    if tick >= last.tick {
        return last.value;
    }
    for pair in points.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if tick >= a.tick && tick <= b.tick {
            let span = (b.tick - a.tick).max(1) as f64;
            let position = (tick - a.tick) as f64 / span;
            return a.value + (b.value - a.value) * position;
        }
    }
    last.value
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

#[cfg(test)]
mod pass_tests {
    use super::{Pass, WriteMode};
    use hardwave_project::automation::{AutomationTarget, CurveMode};
    use hardwave_project::automation_recording::AutomationRecorder;

    fn pass(mode: WriteMode) -> Pass {
        let mut recorder = AutomationRecorder::default();
        recorder.set_mode(mode);
        recorder.on_transport_play();
        recorder.begin_touch();
        Pass {
            target: AutomationTarget::TrackVolume,
            recorder,
            from_play_start: matches!(mode, WriteMode::Write),
        }
    }

    #[test]
    fn touch_ends_at_the_release() {
        let mut p = pass(WriteMode::Touch);
        p.recorder.push_sample(100, 0.2);
        p.recorder.push_sample(200, 0.6);
        assert!(p.release(), "written when let go");
        assert!(!p.recorder.is_recording());
    }

    #[test]
    fn latch_and_write_hold_the_last_value_until_the_stop() {
        for mode in [WriteMode::Latch, WriteMode::Write] {
            let mut p = pass(mode);
            p.recorder.push_sample(100, 0.2);
            p.recorder.push_sample(200, 0.6);
            assert!(!p.release(), "{mode:?} goes on after a release");
            assert!(p.recorder.is_recording());
            p.hold_until(960);
            let points = p.recorder.into_points(CurveMode::Linear);
            let last = points.last().unwrap();
            assert_eq!((last.tick, last.value), (960, 0.6));
        }
    }
}
