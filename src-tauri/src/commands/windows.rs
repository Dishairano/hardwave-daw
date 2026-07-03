//! Detachable panel windows — pop a DAW panel (piano roll, mixer, …) out into
//! its own OS window so it can live on a second monitor.
//!
//! The new window loads the same frontend bundle with `?window=<panel>`; a
//! top-level branch in the UI (`main.tsx` → `PanelWindow`) renders just that
//! panel instead of the full app. State stays live because the engine
//! broadcasts its `daw:*` events with `app.emit` (delivered to ALL windows)
//! and every window talks to the same Rust backend over commands — so this
//! needs no new sync plumbing for playback/meters. Mirrors the proven
//! `WebviewWindowBuilder` usage in `commands/plugins.rs` (plugin editors).

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

fn label_for(panel: &str) -> String {
    // One window per panel id; keep the label to a safe alphanumeric slug so
    // it matches the `panel-*` capability glob.
    let slug: String = panel
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    format!("panel-{slug}")
}

/// Open (or focus) a detached OS window showing a single panel. `panel` is a
/// panel id (pianoRoll | mixer | channelRack | playlist | browser); `params`
/// is an optional extra query string (e.g. "trackId=..&clipId=..") carrying
/// the context the panel needs (the piano roll's open clip).
#[tauri::command]
pub fn open_panel_window(
    app: AppHandle,
    panel: String,
    params: Option<String>,
) -> Result<String, String> {
    let slug: String = panel
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if slug.is_empty() {
        return Err("invalid panel id".into());
    }
    let label = label_for(&panel);

    // Re-clicking detach on an already-open panel just focuses its window.
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(label);
    }

    // Carry the panel id + context via an INITIALIZATION SCRIPT, not the URL.
    // Any query/hash on the window URL can break Tauri's asset resolution so
    // index.html never loads (blank white window — the bug the founder hit).
    // An init script runs before the page loads and sets a global the frontend
    // reads (main.tsx). The URL stays a plain `index.html` that always loads.
    let params_js = params
        .unwrap_or_default()
        .replace('"', "")
        .replace('\\', "");
    let init = format!("window.__HW_PANEL__ = {{ panel: \"{slug}\", params: \"{params_js}\" }};");
    let url = WebviewUrl::App("index.html".into());

    WebviewWindowBuilder::new(&app, &label, url)
        .title(format!("Hardwave DAW — {slug}"))
        .initialization_script(&init)
        .inner_size(1100.0, 720.0)
        .min_inner_size(480.0, 320.0)
        .resizable(true)
        .build()
        .map_err(|e| format!("Failed to open panel window: {e}"))?;

    Ok(label)
}

/// Close a detached panel window (used when a panel is re-docked).
#[tauri::command]
pub fn close_panel_window(app: AppHandle, panel: String) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(&label_for(&panel)) {
        let _ = w.close();
    }
    Ok(())
}
