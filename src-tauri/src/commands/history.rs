use crate::AppState;
use tauri::State;

#[tauri::command]
pub fn undo(state: State<AppState>) -> bool {
    let engine = state.engine.lock();
    let ok = engine.undo();
    if ok {
        engine.rebuild_graph();
    }
    ok
}

#[tauri::command]
pub fn redo(state: State<AppState>) -> bool {
    let engine = state.engine.lock();
    let ok = engine.redo();
    if ok {
        engine.rebuild_graph();
    }
    ok
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
