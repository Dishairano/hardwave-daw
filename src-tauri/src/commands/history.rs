use crate::AppState;
use tauri::State;

#[tauri::command]
pub fn undo(state: State<AppState>) -> bool {
    let mut engine = state.engine.lock();
    // Not while recording: a step that arms or disarms a track would send
    // inputs into the wrong takes.
    if engine
        .transport
        .recording
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return false;
    }
    let ok = engine.undo();
    if ok {
        engine.rebuild_graph();
        // An arm can come back or go away with the step.
        let _ = engine.follow_armed_inputs();
    }
    ok
}

#[tauri::command]
pub fn redo(state: State<AppState>) -> bool {
    let mut engine = state.engine.lock();
    // Not while recording: a step that arms or disarms a track would send
    // inputs into the wrong takes.
    if engine
        .transport
        .recording
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return false;
    }
    let ok = engine.redo();
    if ok {
        engine.rebuild_graph();
        // An arm can come back or go away with the step.
        let _ = engine.follow_armed_inputs();
    }
    ok
}

/// The history list: the engine's undo steps, so the list and undo can
/// never disagree.
#[derive(serde::Serialize)]
pub struct UndoHistory {
    pub entries: Vec<hardwave_engine::engine::HistoryEntry>,
    /// How many of the entries are applied; the rest can be redone.
    pub cursor: usize,
    /// Whether undoing every step gets back to the song as it was opened
    /// (false once old steps have gone past the history's cap).
    #[serde(rename = "startsAtOpen")]
    pub starts_at_open: bool,
}

#[tauri::command]
pub fn get_undo_history(state: State<AppState>) -> UndoHistory {
    // The engine's own lock only long enough to reach the two parts.
    let (project, history) = {
        let engine = state.engine.lock();
        (
            std::sync::Arc::clone(&engine.project),
            std::sync::Arc::clone(&engine.history),
        )
    };
    let (entries, cursor, starts_at_open) =
        hardwave_engine::DawEngine::history_entries(&project, &history);
    UndoHistory {
        entries,
        cursor,
        starts_at_open,
    }
}

/// Name the edit a command just made ("Mute clip"). `since` is when the
/// app started that command, in milliseconds since 1970: a command that
/// took no step finds only older ones, and its name is dropped rather
/// than landing on one of them. Steps nobody names are described from
/// the song.
#[tauri::command]
pub fn label_undo_step(state: State<AppState>, label: String, since: u64) -> bool {
    state.engine.lock().label_newest_edit(label, since)
}

/// Forget every undo step: the history list's "Clear".
#[tauri::command]
pub fn clear_undo_history(state: State<AppState>) {
    state.engine.lock().reset_history();
}

/// Returns `(undo_depth, redo_depth)` so the UI can grey out buttons.
#[tauri::command]
pub fn history_sizes(state: State<AppState>) -> (usize, usize) {
    state.engine.lock().history_sizes()
}

/// Treat the mutations that follow as one undo step, until
/// `end_history_group`.
///
/// One gesture should be one undo. Painting clips across the playlist places
/// each clip through its own command, so without this a single drag left as
/// many undo steps as it placed clips.
#[tauri::command]
pub fn begin_history_group(state: State<AppState>) {
    state.engine.lock().begin_history_group();
}

#[tauri::command]
pub fn end_history_group(state: State<AppState>) {
    state.engine.lock().end_history_group();
}
