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
    // Comprehensive REMOTE-LOG diagnostic (init script runs before the bundle,
    // so it reports even if the frontend never loads). Beacons the window's
    // real URL, JS errors, DOMContentLoaded, and whether React mounted to our
    // collector (suite.hardwavestudios.com/daw-log) so we can diagnose the
    // blank-white detach without devtools. Temporary — remove when solved.
    let log_url = "https://suite.hardwavestudios.com/daw-log";
    let init = format!(
        "(function(){{var P=\"{slug}\";var U=\"{log_url}\";\
         window.__HW_PANEL__={{panel:P,params:\"{params_js}\"}};\
         function S(t,d){{try{{fetch(U,{{method:'POST',mode:'no-cors',headers:{{'Content-Type':'text/plain'}},body:JSON.stringify({{t:t,panel:P,href:location.href,data:d}})}});}}catch(e){{}}}}\
         window.__HW_LOG__=S;S('init',{{rs:document.readyState,ua:navigator.userAgent.slice(0,50)}});\
         addEventListener('error',function(e){{S('error',{{m:''+(e.message||''),s:''+(e.filename||''),l:e.lineno,stk:(''+((e.error&&e.error.stack)||'')).slice(0,500)}});}});\
         addEventListener('unhandledrejection',function(e){{S('reject',{{r:(''+((e.reason&&(e.reason.stack||e.reason.message))||e.reason||'')).slice(0,500)}});}});\
         addEventListener('DOMContentLoaded',function(){{S('dom',{{title:document.title,scripts:document.scripts.length}});\
           setTimeout(function(){{var r=document.getElementById('root');\
             S('mount',{{root:r?r.childElementCount:'no-root',bodyLen:document.body?document.body.innerHTML.length:0}});\
             if(r&&r.childElementCount===0){{document.body.style.background='#1a0008';document.body.innerHTML='<pre style=\"color:#ff8a8a;padding:16px;font:12px monospace;white-space:pre-wrap\">Panel loaded but the app did not mount (remote log sent). panel='+P+'</pre>';}}\
           }},3000);}});}})();"
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

    WebviewWindowBuilder::new(&app, &label, url)
        .title(format!("Hardwave DAW — {slug}"))
        // Frameless like the main DAW window (no OS "Hardwave DAW" title bar);
        // the panel renders its own thin drag/close bar.
        .decorations(false)
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
pub async fn close_panel_window(app: AppHandle, panel: String) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(&label_for(&panel)) {
        let _ = w.close();
    }
    Ok(())
}
