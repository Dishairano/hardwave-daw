//! Send routing commands. A send taps a source track's signal (pre- or
//! post-fader) and feeds it into another track's input with a per-edge gain.
//! The target track is typically a bus that hosts a reverb/delay plugin (a
//! "return track"), but any track can be a send target.

use crate::AppState;
use hardwave_project::mixer::Send as ProjectSend;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct SendInfo {
    pub index: usize,
    pub target: String,
    #[serde(rename = "gainDb")]
    pub gain_db: f64,
    #[serde(rename = "preFader")]
    pub pre_fader: bool,
    pub enabled: bool,
}

fn sends_of(track: &hardwave_project::Track) -> Vec<SendInfo> {
    track
        .sends
        .iter()
        .enumerate()
        .map(|(index, s)| SendInfo {
            index,
            target: s.target.clone(),
            gain_db: s.gain_db,
            pre_fader: s.pre_fader,
            enabled: s.enabled,
        })
        .collect()
}

#[tauri::command]
pub fn get_sends(state: State<AppState>, track_id: String) -> Vec<SendInfo> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project.track(&track_id).map(sends_of).unwrap_or_default()
}

/// List every send across every track. Used by the mixer to paint "visual
/// send indicators" (arrows from source to target strips).
#[derive(Serialize)]
pub struct SendEdge {
    pub source: String,
    pub target: String,
    pub index: usize,
    #[serde(rename = "gainDb")]
    pub gain_db: f64,
    #[serde(rename = "preFader")]
    pub pre_fader: bool,
    pub enabled: bool,
}

#[tauri::command]
pub fn list_sends(state: State<AppState>) -> Vec<SendEdge> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    let mut out = Vec::new();
    for track in &project.tracks {
        for (i, s) in track.sends.iter().enumerate() {
            out.push(SendEdge {
                source: track.id.clone(),
                target: s.target.clone(),
                index: i,
                gain_db: s.gain_db,
                pre_fader: s.pre_fader,
                enabled: s.enabled,
            });
        }
    }
    out
}

/// Returns true if `src` can reach `dst` by following existing send edges.
/// Used to reject cycles at add-time so the topological sort always terminates.
fn send_path_exists(project: &hardwave_project::Project, src: &str, dst: &str) -> bool {
    let mut stack = vec![src.to_string()];
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    while let Some(cur) = stack.pop() {
        if cur == dst {
            return true;
        }
        if !seen.insert(cur.clone()) {
            continue;
        }
        if let Some(track) = project.track(&cur) {
            for send in &track.sends {
                if send.enabled {
                    stack.push(send.target.clone());
                }
            }
        }
    }
    false
}

#[tauri::command]
pub fn add_send(
    state: State<AppState>,
    track_id: String,
    target_id: String,
    gain_db: Option<f64>,
    pre_fader: Option<bool>,
) -> Result<usize, String> {
    if track_id == target_id {
        return Err("A track cannot send to itself".into());
    }
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let idx = {
        let mut project = engine.project.lock();
        if project.track(&track_id).is_none() {
            return Err(format!("Source track not found: {track_id}"));
        }
        if project.track(&target_id).is_none() {
            return Err(format!("Target track not found: {target_id}"));
        }
        // Reject cycles: target must not already reach back to source.
        if send_path_exists(&project, &target_id, &track_id) {
            return Err("Refusing to create a send cycle".into());
        }
        let gain = gain_db.unwrap_or(0.0).clamp(-100.0, 6.0);
        let pre = pre_fader.unwrap_or(false);
        let send = ProjectSend {
            target: target_id,
            gain_db: gain,
            pre_fader: pre,
            enabled: true,
        };
        let track = project.track_mut(&track_id).unwrap();
        track.sends.push(send);
        track.sends.len() - 1
    };
    engine.rebuild_graph();
    Ok(idx)
}

#[tauri::command]
pub fn remove_send(state: State<AppState>, track_id: String, send_index: usize) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            if send_index < track.sends.len() {
                track.sends.remove(send_index);
            }
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_send_target(
    state: State<AppState>,
    track_id: String,
    send_index: usize,
    target_id: String,
) -> Result<(), String> {
    if track_id == target_id {
        return Err("A track cannot send to itself".into());
    }
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if project.track(&target_id).is_none() {
            return Err(format!("Target track not found: {target_id}"));
        }
        // Cycle check using the proposed target.
        if send_path_exists(&project, &target_id, &track_id) {
            return Err("Refusing to create a send cycle".into());
        }
        if let Some(track) = project.track_mut(&track_id) {
            if let Some(send) = track.sends.get_mut(send_index) {
                send.target = target_id;
            }
        }
    }
    engine.rebuild_graph();
    Ok(())
}

#[tauri::command]
pub fn set_send_gain(
    state: State<AppState>,
    track_id: String,
    send_index: usize,
    gain_db: Option<f64>,
) {
    let Some(gain_db) = gain_db else { return };
    if !gain_db.is_finite() {
        return;
    }
    let gain_db = gain_db.clamp(-100.0, 6.0);
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            if let Some(send) = track.sends.get_mut(send_index) {
                send.gain_db = gain_db;
            }
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_send_pre_fader(
    state: State<AppState>,
    track_id: String,
    send_index: usize,
    pre_fader: bool,
) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            if let Some(send) = track.sends.get_mut(send_index) {
                send.pre_fader = pre_fader;
            }
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_send_enabled(
    state: State<AppState>,
    track_id: String,
    send_index: usize,
    enabled: bool,
) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            if let Some(send) = track.sends.get_mut(send_index) {
                send.enabled = enabled;
            }
        }
    }
    engine.rebuild_graph();
}

/// Convenience: create a new `Return`-kind track that hosts reverb or delay
/// plugins and a send from the caller into it. Returns the new return track id.
#[tauri::command]
pub fn create_return_with_send(
    state: State<AppState>,
    source_track_id: String,
    return_name: String,
) -> Result<String, String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let new_id = {
        let mut project = engine.project.lock();
        if project.track(&source_track_id).is_none() {
            return Err(format!("Source track not found: {source_track_id}"));
        }
        // Insert the new return track before the master so rebuild_graph
        // processes it in the same block.
        let new_id = project.add_audio_track(return_name);
        // Re-kind it as Return so the UI can style it distinctly.
        if let Some(new_track) = project.track_mut(&new_id) {
            new_track.kind = hardwave_project::track::TrackKind::Return;
            new_track.color = "#22d3ee".into();
        }
        // Add the send from the source → new return, post-fader, unity.
        if let Some(src) = project.track_mut(&source_track_id) {
            src.sends.push(ProjectSend {
                target: new_id.clone(),
                gain_db: 0.0,
                pre_fader: false,
                enabled: true,
            });
        }
        new_id
    };
    engine.sync_track_meters();
    engine.rebuild_graph();
    Ok(new_id)
}
