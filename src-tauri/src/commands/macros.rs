//! Macros: one knob, several parameters.
//!
//! The model lives in `hardwave_project::macros`. This file is the part
//! that talks to the engine: turning a macro is a live change, so every
//! link is pushed to the audio thread the same way a hand on that knob
//! would push it.

use crate::AppState;
use hardwave_engine::insert_chain::InsertCommand;
use hardwave_project::macros::{Macro, MacroLink, MacroTarget};
use tauri::State;

/// Push every link of one macro at its current value.
///
/// Track volume and pan are project state, so they are written and the
/// graph is rebuilt. Plug-in parameters are sent as insert commands, which
/// is the path a live knob already uses; a full queue is not an error worth
/// stopping the whole macro for, so it is skipped and the rest still moves.
fn apply(state: &State<AppState>, the_macro: &Macro) {
    let mut touched_project = false;
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        for link in &the_macro.links {
            let value = link.value_at(the_macro.value);
            match &link.target {
                MacroTarget::TrackVolume => {
                    if let Some(track) = project.track_mut(&link.track_id) {
                        track.volume_db = value;
                        touched_project = true;
                    }
                }
                MacroTarget::TrackPan => {
                    if let Some(track) = project.track_mut(&link.track_id) {
                        track.pan = value.clamp(-1.0, 1.0);
                        touched_project = true;
                    }
                }
                MacroTarget::PluginParam { slot_id, param_id } => {
                    let cmd = InsertCommand::SetParameter {
                        track_id: link.track_id.clone(),
                        slot_id: slot_id.clone(),
                        param_id: *param_id,
                        value,
                    };
                    let _ = engine.try_send_insert_command(cmd);
                }
            }
        }
    }
    if touched_project {
        state.engine.lock().rebuild_graph();
    }
}

fn with_macro<T>(
    state: &State<AppState>,
    id: &str,
    f: impl FnOnce(&mut Macro) -> T,
) -> Result<T, String> {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let the_macro = project
        .macros
        .iter_mut()
        .find(|m| m.id == id)
        .ok_or_else(|| "no macro with that id".to_string())?;
    Ok(f(the_macro))
}

#[tauri::command]
pub fn list_macros(state: State<AppState>) -> Vec<Macro> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project.macros.clone()
}

#[tauri::command]
pub fn add_macro(state: State<AppState>, name: String) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("give the macro a name".into());
    }
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let id = uuid::Uuid::new_v4().to_string();
    project.macros.push(Macro::new(id.clone(), name));
    Ok(id)
}

#[tauri::command]
pub fn rename_macro(state: State<AppState>, id: String, name: String) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("give the macro a name".into());
    }
    state.engine.lock().snapshot_before_mutation();
    with_macro(&state, &id, |m| m.name = name)
}

#[tauri::command]
pub fn delete_macro(state: State<AppState>, id: String) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let before = project.macros.len();
    project.macros.retain(|m| m.id != id);
    if project.macros.len() == before {
        return Err("no macro with that id".into());
    }
    Ok(())
}

/// Turn the knob.
///
/// No undo snapshot: this is a knob being moved, and filling the history
/// with every step of a sweep would make Ctrl+Z useless. The value is saved
/// with the song, so where it was left is where it opens.
#[tauri::command]
pub fn set_macro_value(state: State<AppState>, id: String, value: f64) -> Result<(), String> {
    if !value.is_finite() {
        return Err("that is not a value".into());
    }
    let the_macro = {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let m = project
            .macros
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| "no macro with that id".to_string())?;
        m.value = value.clamp(0.0, 1.0);
        m.clone()
    };
    apply(&state, &the_macro);
    Ok(())
}

/// Link a target to a macro.
///
/// `min` and `max` are the target's own values, not a normalized pair, so
/// a cutoff link is given hertz and a volume link decibels. Putting the
/// larger number in `min` is how a link is made to run backwards.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn add_macro_link(
    state: State<AppState>,
    id: String,
    track_id: String,
    slot_id: Option<String>,
    param_id: Option<u32>,
    track_target: Option<String>,
    min: f64,
    max: f64,
) -> Result<String, String> {
    if !min.is_finite() || !max.is_finite() {
        return Err("give the link two real values".into());
    }
    let target = match (slot_id, param_id, track_target.as_deref()) {
        (Some(slot_id), Some(param_id), _) => MacroTarget::PluginParam { slot_id, param_id },
        (_, _, Some("volume")) => MacroTarget::TrackVolume,
        (_, _, Some("pan")) => MacroTarget::TrackPan,
        _ => return Err("say what the link moves".into()),
    };
    state.engine.lock().snapshot_before_mutation();
    let link_id = uuid::Uuid::new_v4().to_string();
    with_macro(&state, &id, |m| {
        m.links.push(MacroLink {
            id: link_id.clone(),
            track_id,
            target,
            min,
            max,
        });
    })?;
    Ok(link_id)
}

#[tauri::command]
pub fn remove_macro_link(
    state: State<AppState>,
    id: String,
    link_id: String,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    with_macro(&state, &id, |m| m.links.retain(|l| l.id != link_id))
}

#[tauri::command]
pub fn set_macro_link_range(
    state: State<AppState>,
    id: String,
    link_id: String,
    min: f64,
    max: f64,
) -> Result<(), String> {
    if !min.is_finite() || !max.is_finite() {
        return Err("give the link two real values".into());
    }
    state.engine.lock().snapshot_before_mutation();
    let the_macro = with_macro(&state, &id, |m| {
        if let Some(link) = m.links.iter_mut().find(|l| l.id == link_id) {
            link.min = min;
            link.max = max;
        }
        m.clone()
    })?;
    apply(&state, &the_macro);
    Ok(())
}

/// Push every macro at its saved value.
///
/// Called after a song is opened, because the links were written into the
/// engine last time by the knob and the engine has just been rebuilt from
/// the file.
#[tauri::command]
pub fn apply_all_macros(state: State<AppState>) {
    let all = {
        let engine = state.engine.lock();
        let project = engine.project.lock();
        project.macros.clone()
    };
    for the_macro in &all {
        apply(&state, the_macro);
    }
}
