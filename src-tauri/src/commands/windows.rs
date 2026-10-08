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
// ASYNC IS LOAD-BEARING on Windows: synchronous commands run on the main
// thread, and creating a WebView2 there deadlocks the new webview's
// initialization (it needs the main thread to pump its creation messages) —
// the window opens as an HWND but stays blank white forever and never loads
// its URL (which is why not even the init-script beacons fired). An async
// command runs on the async runtime instead, leaving the main thread free.
// Same reason the docs say "you must use async commands when creating
// windows on Windows". Plugin editors got away with a sync command only
// because the native plugin GUI paints over their (equally dead) webview.
#[tauri::command]
pub async fn open_panel_window(
    app: AppHandle,
    panel: String,
    params: Option<String>,
    instance: Option<String>,
    title: Option<String>,
    size: Option<(f64, f64)>,
) -> Result<String, String> {
    let slug: String = panel
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if slug.is_empty() {
        return Err("invalid panel id".into());
    }
    // A panel that can be open more than once (a plug-in's controls, one
    // window per slot) passes an instance; its label stays inside panel-*.
    let instance_slug: String = instance
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(64)
        .collect();
    let label = if instance_slug.is_empty() {
        label_for(&panel)
    } else {
        format!("{}-{instance_slug}", label_for(&panel))
    };
    let plugin_window = slug == "pluginControls";

    // Re-clicking detach on an already-open panel just focuses its window.
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(label);
    }

    let params_js = params.unwrap_or_default().replace(['"', '\\'], "");
    // The panel id and its context reach the page through an INITIALIZATION
    // SCRIPT, not the URL. Any query or hash on the window URL can break
    // Tauri's asset resolution, so index.html never loads and the window
    // stays blank white. An init script runs before the page and sets a
    // global the frontend reads (main.tsx).
    //
    // This used to carry a diagnostic that posted the window's URL, its
    // errors and whether React mounted to a collector on our server. It was
    // written to find the blank-white window, that is solved, and shipping
    // it to other people would send their machine's activity to us without
    // saying so.
    let init = format!(
        "(function(){{window.__HW_PANEL__={{panel:\"{slug}\",params:\"{params_js}\"}};}})();"
    );

    // Load the EXACT url the main window is showing rather than guessing
    // `index.html`. On WebView2 the main window serves from a specific origin
    // (http://tauri.localhost/), and a fresh `WebviewUrl::App("index.html")`
    // second window was coming up blank white. Cloning the main window's live
    // URL guarantees the panel window loads the same frontend that already
    // renders in the main window.
    let url = app
        .get_webview_window("main")
        .and_then(|w| w.url().ok())
        .map(WebviewUrl::External)
        .unwrap_or_else(|| WebviewUrl::App("index.html".into()));

    let window_title = title
        .map(|t| {
            t.chars()
                .filter(|c| !c.is_control())
                .take(80)
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| format!("Hardwave DAW - {slug}"));
    let (w, h, min_w, min_h) = if plugin_window {
        // A built-in's window knows its size (its layout's width); the page
        // fits the height to what it shows once it has drawn.
        let (w, h) = size.unwrap_or((380.0, 560.0));
        (w.clamp(300.0, 2400.0), h.clamp(240.0, 1600.0), 300.0, 240.0)
    } else {
        (1100.0, 720.0, 480.0, 320.0)
    };
    WebviewWindowBuilder::new(&app, &label, url)
        .title(window_title)
        // Frameless like the main DAW window (no OS "Hardwave DAW" title bar);
        // the panel renders its own thin drag/close bar.
        .decorations(false)
        .initialization_script(&init)
        .inner_size(w, h)
        .min_inner_size(min_w, min_h)
        .resizable(true)
        .build()
        .map_err(|e| format!("Failed to open panel window: {e}"))?;

    Ok(label)
}

/// Close a detached panel window (used when a panel is re-docked).
#[tauri::command]
pub async fn close_panel_window(app: AppHandle, panel: String) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(&label_for(&panel)) {
        let _ = w.close();
    }
    Ok(())
}

/// Size the calling panel window to what its page shows (a built-in plug-in's
/// window measures itself after drawing). Only panel windows: the main
/// window keeps the size the user gave it.
#[tauri::command]
pub fn fit_panel_window(
    window: tauri::WebviewWindow,
    width: f64,
    height: f64,
) -> Result<(), String> {
    if !window.label().starts_with("panel-") {
        return Err("only a panel window can fit itself".into());
    }
    window
        .set_size(tauri::LogicalSize::new(
            width.clamp(300.0, 2400.0),
            height.clamp(200.0, 1600.0),
        ))
        .map_err(|e| e.to_string())
}
