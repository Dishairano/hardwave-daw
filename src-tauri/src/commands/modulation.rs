//! The modulation matrix: sources wired to plug-in knobs.
//!
//! The arithmetic lives in `hardwave_project::modulation`; the engine
//! reads the routes every block. This is the part the app calls.

use crate::AppState;
use hardwave_project::lfo::{LfoRate, LfoShape};
use hardwave_project::modulation::{ModRoute, ModSource, ModTarget};
use tauri::State;

#[tauri::command]
pub fn list_modulations(state: State<AppState>) -> Vec<ModRoute> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project.modulations.clone()
}

/// Wire a source to a knob.
///
/// The centre is where the knob sits with the source in the middle, and
/// the depth is how far either side it swings, so a route added to a
/// knob that is already where you want it changes nothing until the
/// depth is turned up.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn add_modulation(
    state: State<AppState>,
    name: String,
    track_id: String,
    slot_id: String,
    param_id: u32,
    shape: String,
    sync_num: Option<u32>,
    sync_den: Option<u32>,
    hz: Option<f64>,
    center: f64,
    depth: f64,
) -> Result<String, String> {
    let shape = match shape.as_str() {
        "sine" => LfoShape::Sine,
        "triangle" => LfoShape::Triangle,
        "square" => LfoShape::Square,
        "saw_up" => LfoShape::SawtoothUp,
        "saw_down" => LfoShape::SawtoothDown,
        "random" => LfoShape::RandomSampleAndHold,
        other => return Err(format!("no shape called {other}")),
    };
    let rate = match (sync_num, sync_den, hz) {
        (Some(num), Some(den), _) if num > 0 && den > 0 => LfoRate::TempoSync { num, den },
        (_, _, Some(hz)) if hz > 0.0 => LfoRate::Hz(hz),
        _ => return Err("give the source a rate".into()),
    };
    state.engine.lock().snapshot_before_mutation();
    let id = uuid::Uuid::new_v4().to_string();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        if project.track(&track_id).is_none() {
            return Err(format!("Track not found: {track_id}"));
        }
        project.modulations.push(ModRoute {
            id: id.clone(),
            name: if name.trim().is_empty() {
                "Modulation".to_string()
            } else {
                name.trim().to_string()
            },
            enabled: true,
            source: ModSource::Lfo {
                shape,
                rate,
                phase_offset: 0.0,
            },
            track_id,
            target: ModTarget::PluginParam { slot_id, param_id },
            center: center.clamp(0.0, 1.0),
            depth: depth.clamp(-1.0, 1.0),
        });
    }
    Ok(id)
}

/// Move a route's centre, depth or switch, without a history step: these
/// are knobs, and a sweep of one would fill the undo list.
#[tauri::command]
pub fn set_modulation(
    state: State<AppState>,
    id: String,
    center: Option<f64>,
    depth: Option<f64>,
    enabled: Option<bool>,
    phase_offset: Option<f64>,
) -> Result<(), String> {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let route = project
        .modulations
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or_else(|| "no modulation with that id".to_string())?;
    if let Some(c) = center {
        route.center = c.clamp(0.0, 1.0);
    }
    if let Some(d) = depth {
        route.depth = d.clamp(-1.0, 1.0);
    }
    if let Some(e) = enabled {
        route.enabled = e;
    }
    if let Some(p) = phase_offset {
        let ModSource::Lfo {
            ref mut phase_offset,
            ..
        } = route.source;
        *phase_offset = p.rem_euclid(1.0);
    }
    Ok(())
}

#[tauri::command]
pub fn delete_modulation(state: State<AppState>, id: String) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let before = project.modulations.len();
    project.modulations.retain(|r| r.id != id);
    if project.modulations.len() == before {
        return Err("no modulation with that id".into());
    }
    Ok(())
}
