//! The modulation matrix: sources wired to plug-in knobs and to a track's
//! volume, pan and sends.
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
///
/// The source is an LFO (`shape` and a rate) unless `source` says
/// `envelope` (the level of `source_track_id`, falling back over
/// `release_ms`) or `cc` (controller `cc`). The target is a plug-in's knob
/// (`slot_id` and `param_id`) unless `target` says `volume`, `pan` or
/// `send` (`send_index`).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn add_modulation(
    state: State<AppState>,
    name: String,
    track_id: String,
    slot_id: Option<String>,
    param_id: Option<u32>,
    shape: Option<String>,
    sync_num: Option<u32>,
    sync_den: Option<u32>,
    hz: Option<f64>,
    center: f64,
    depth: f64,
    source: Option<String>,
    source_track_id: Option<String>,
    release_ms: Option<f64>,
    cc: Option<u8>,
    target: Option<String>,
    send_index: Option<usize>,
) -> Result<String, String> {
    let source = match source.as_deref().unwrap_or("lfo") {
        "lfo" => {
            let shape = match shape.as_deref().unwrap_or("sine") {
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
            ModSource::Lfo {
                shape,
                rate,
                phase_offset: 0.0,
            }
        }
        "envelope" => ModSource::Envelope {
            track_id: source_track_id.ok_or("pick the track whose level it follows")?,
            release_ms: release_ms.unwrap_or(120.0).clamp(1.0, 5_000.0),
        },
        "cc" => ModSource::MidiCc {
            cc: cc
                .filter(|c| *c < 128)
                .ok_or("pick a controller from 0 to 127")?,
        },
        other => return Err(format!("no source called {other}")),
    };
    let target = match target.as_deref().unwrap_or("plugin") {
        "plugin" => ModTarget::PluginParam {
            slot_id: slot_id.ok_or("pick a plug-in")?,
            param_id: param_id.ok_or("pick a knob")?,
        },
        "volume" => ModTarget::TrackVolume,
        "pan" => ModTarget::TrackPan,
        "send" => ModTarget::SendLevel {
            send_index: send_index.ok_or("pick a send")?,
        },
        other => return Err(format!("nothing to move called {other}")),
    };
    if let ModSource::Envelope {
        track_id: ref followed,
        ..
    } = source
    {
        // Its own level turning its own volume down turns the level down,
        // which turns the volume back up: a wobble, not a duck.
        if *followed == track_id && target == ModTarget::TrackVolume {
            return Err("a track's volume can't follow its own level".into());
        }
    }
    state.engine.lock().snapshot_before_mutation();
    let id = uuid::Uuid::new_v4().to_string();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        if project.track(&track_id).is_none() {
            return Err(format!("Track not found: {track_id}"));
        }
        if let ModSource::Envelope {
            track_id: ref followed,
            ..
        } = source
        {
            if project.track(followed).is_none() {
                return Err(format!("Track not found: {followed}"));
            }
        }
        if let ModTarget::SendLevel { send_index } = target {
            let sends = project.track(&track_id).map_or(0, |t| t.sends.len());
            if send_index >= sends {
                return Err("that track has no such send".into());
            }
        }
        project.modulations.push(ModRoute {
            id: id.clone(),
            name: if name.trim().is_empty() {
                "Modulation".to_string()
            } else {
                name.trim().to_string()
            },
            enabled: true,
            source,
            track_id,
            target,
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
        if let ModSource::Lfo {
            ref mut phase_offset,
            ..
        } = route.source
        {
            *phase_offset = p.rem_euclid(1.0);
        }
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
