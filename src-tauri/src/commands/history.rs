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
}

#[tauri::command]
pub fn get_undo_history(state: State<AppState>) -> UndoHistory {
    let (entries, cursor) = state.engine.lock().history_entries();
    UndoHistory { entries, cursor }
}

/// Name the edit the command before this one just made ("Mute clip").
/// A name that comes with no new edit is dropped, so it never lands on
/// an older step. Steps nobody names are described from the song.
#[tauri::command]
pub fn label_undo_step(state: State<AppState>, label: String) -> bool {
    state.engine.lock().label_newest_edit(label)
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
