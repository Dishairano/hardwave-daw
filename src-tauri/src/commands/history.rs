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
