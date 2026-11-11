//! The mixer by number: making an empty insert a track, and routing
//! channels to an insert by its number, as FL's insert box does.
//!
//! Each command is one undo step. What they change is tracks (a new bus,
//! a track's output), and a room sends every changed or new track whole,
//! so both travel with nothing added here.

use crate::AppState;
use hardwave_engine::DawEngine;
use tauri::State;

/// Make insert `number` a track (an empty bus into the master), named
/// `name` or "Insert N". Used when an effect, a send or a name is put on
/// an empty number in the mixer. Returns the insert's id; one that is
/// already there is returned unchanged, with no undo step.
#[tauri::command]
pub fn make_insert(
    state: State<AppState>,
    number: u16,
    name: Option<String>,
) -> Result<String, String> {
    make_insert_in(&state.engine.lock(), number, name.as_deref())
}

pub(crate) fn make_insert_in(
    engine: &DawEngine,
    number: u16,
    name: Option<&str>,
) -> Result<String, String> {
    if let Some(track) = engine.project.lock().insert_numbered(number) {
        return Ok(track.id.clone());
    }
    engine.snapshot_before_mutation();
    let (id, _) = engine.project.lock().make_insert(number, name)?;
    engine.sync_track_meters();
    engine.rebuild_graph();
    Ok(id)
}

/// Send each of `track_ids` to insert `number` (0 the master). An empty
/// number becomes an insert first. All of it is one undo step; when one
/// route would play back into itself nothing changes and the reason comes
/// back. Returns the id of an insert made for it.
#[tauri::command]
pub fn route_to_insert(
    state: State<AppState>,
    track_ids: Vec<String>,
    number: u16,
) -> Result<Option<String>, String> {
    route_to_insert_in(&state.engine.lock(), &track_ids, number)
}

pub(crate) fn route_to_insert_in(
    engine: &DawEngine,
    track_ids: &[String],
    number: u16,
) -> Result<Option<String>, String> {
    {
        let project = engine.project.lock();
        for id in track_ids {
            project.check_route_to_insert(id, number)?;
        }
    }
    if track_ids.is_empty() {
        return Ok(None);
    }
    engine.snapshot_before_mutation();
    let mut made = None;
    {
        let mut project = engine.project.lock();
        for id in track_ids {
            if let Some(new) = project.route_to_insert(id, number)? {
                made = Some(new);
            }
        }
    }
    if made.is_some() {
        engine.sync_track_meters();
    }
    engine.rebuild_graph();
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::Collab;
    use hardwave_project::Project;

    fn engine() -> (DawEngine, String, String) {
        let engine = DawEngine::new();
        let (kick, hat) = {
            let mut p = engine.project.lock();
            (
                p.add_audio_track("Kick".into()),
                p.add_audio_track("Hat".into()),
            )
        };
        (engine, kick, hat)
    }

    fn steps(engine: &DawEngine) -> usize {
        engine.history_sizes().0
    }

    #[test]
    fn making_an_insert_is_one_undo_step() {
        let (engine, _, _) = engine();
        let before = steps(&engine);
        let tracks = engine.project.lock().tracks.len();
        // A new song's numbers go to 500, then the two added tracks.
        let id = make_insert_in(&engine, 600, Some("Verb")).unwrap();
        assert_eq!(steps(&engine), before + 1);
        assert_eq!(engine.project.lock().insert_numbered(600).unwrap().id, id);
        assert_eq!(make_insert_in(&engine, 600, None).unwrap(), id);
        assert_eq!(steps(&engine), before + 1, "one that is there: no step");
        assert!(engine.undo());
        assert_eq!(engine.project.lock().tracks.len(), tracks);
        assert!(engine.project.lock().insert_numbered(600).is_none());
    }

    #[test]
    fn routing_to_an_empty_number_is_one_undo_step_for_every_channel() {
        let (engine, kick, hat) = engine();
        let before = steps(&engine);
        let tracks = engine.project.lock().tracks.len();
        let made = route_to_insert_in(&engine, &[kick.clone(), hat.clone()], 700)
            .unwrap()
            .expect("700 was empty");
        {
            let p = engine.project.lock();
            assert_eq!(p.track(&kick).unwrap().output_bus.as_ref(), Some(&made));
            assert_eq!(p.track(&hat).unwrap().output_bus.as_ref(), Some(&made));
        }
        assert_eq!(steps(&engine), before + 1);
        assert!(engine.undo());
        let p = engine.project.lock();
        assert_eq!(p.tracks.len(), tracks, "the insert goes with the step");
        assert_eq!(p.track(&kick).unwrap().output_bus, None);
        assert_eq!(p.track(&hat).unwrap().output_bus, None);
    }

    #[test]
    fn a_refused_route_changes_nothing_and_takes_no_step() {
        let (engine, kick, _) = engine();
        route_to_insert_in(&engine, std::slice::from_ref(&kick), 3).unwrap();
        let three = engine.project.lock().insert_numbered(3).unwrap().id.clone();
        let kick_number = engine
            .project
            .lock()
            .track(&kick)
            .unwrap()
            .mixer_number
            .unwrap();
        let before = steps(&engine);
        let song = engine.project.lock().tracks.clone();
        // Insert 3 into the kick's own insert, which plays into 3: a loop.
        let err = route_to_insert_in(&engine, &[three], kick_number).unwrap_err();
        assert!(err.contains("cycle"), "{err}");
        assert_eq!(steps(&engine), before);
        assert_eq!(engine.project.lock().tracks, song);
    }

    /// What one side's command makes reaches the other side of a room:
    /// the new insert with its number, and the channel's route to it.
    #[test]
    fn a_new_insert_and_the_route_to_it_travel_in_a_room() {
        let (engine, kick, _) = engine();
        let here = Collab::default();
        let start = engine.project.lock().clone();
        here.remember_tracks(&start);
        let mut there: Project = start.clone();

        let made = route_to_insert_in(&engine, std::slice::from_ref(&kick), 640)
            .unwrap()
            .unwrap();
        let named = make_insert_in(&engine, 641, Some("KICK VERB")).unwrap();
        let (changed, gone) = here.track_changes(&engine.project.lock());
        assert!(gone.is_empty());
        let ids: Vec<&str> = changed.iter().map(|(id, _, _)| id.as_str()).collect();
        assert!(ids.contains(&kick.as_str()) && ids.contains(&made.as_str()));
        assert!(ids.contains(&named.as_str()));
        for (_, index, blob) in &changed {
            there.put_track_blob(blob, *index).unwrap();
        }
        assert_eq!(there.insert_numbered(640).unwrap().id, made);
        assert_eq!(there.insert_numbered(641).unwrap().name, "KICK VERB");
        assert_eq!(there.track(&kick).unwrap().output_bus.as_ref(), Some(&made));
        assert_eq!(there.tracks, engine.project.lock().tracks);
    }
}
