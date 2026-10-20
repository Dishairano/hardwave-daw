//! Plug-in windows kept ready for the next editor (Windows).
//!
//! A plug-in's window carries our bar along its top, a page of the app, so making one builds a
//! WebView2. Tauri builds it on the main thread and waits for it: on the test VM the playhead
//! stood still 0.7 to 4 s each time an editor opened (20 Oct 2026), while the plug-in's own
//! part took under 0.1 s. So one window is made ahead, while the app starts, and a window that
//! closes is hidden and kept for the next editor instead of being destroyed. The bar is told
//! which plug-in it now belongs to (`daw:pluginHeader`).

// Only Windows builds use the pool in the app; the tests use it everywhere.
#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::HashMap;

/// Closed windows kept for later; beyond this a closed window really closes.
const KEEP: usize = 3;

/// The windows' bookkeeping, apart from the windows themselves so it can be tested.
#[derive(Default)]
struct Pool {
    /// Hidden windows ready for an editor.
    ready: Vec<String>,
    /// The open window of each slot.
    by_slot: HashMap<String, String>,
    next: u32,
}

/// What happens to a window whose editor closed.
#[derive(Debug, PartialEq, Eq)]
enum Release {
    /// Hide it and keep it for the next editor.
    Keep,
    /// Let it close.
    Close,
}

impl Pool {
    fn new_label(&mut self) -> String {
        self.next += 1;
        format!("panel-pluginEditor-w{}", self.next)
    }

    fn open_for(&self, slot: &str) -> Option<String> {
        self.by_slot.get(slot).cloned()
    }

    fn ready(&mut self, label: &str) {
        self.ready.push(label.to_string());
    }

    fn take_ready(&mut self) -> Option<String> {
        self.ready.pop()
    }

    fn assign(&mut self, slot: &str, label: &str) {
        self.by_slot.insert(slot.to_string(), label.to_string());
    }

    fn release(&mut self, label: &str) -> Release {
        self.by_slot.retain(|_, l| l != label);
        if self.ready.len() < KEEP && !self.ready.iter().any(|l| l == label) {
            self.ready.push(label.to_string());
            Release::Keep
        } else {
            Release::Close
        }
    }

    fn forget(&mut self, label: &str) {
        self.by_slot.retain(|_, l| l != label);
        self.ready.retain(|l| l != label);
    }

    fn all(&self) -> Vec<String> {
        let mut labels: Vec<String> = self.by_slot.values().cloned().collect();
        labels.extend(self.ready.iter().cloned());
        labels
    }
}

#[cfg(windows)]
pub use windows_pool::{close_all, editor_opened, open_window, prepare, take_or_make};

#[cfg(windows)]
mod windows_pool {
    use super::{Pool, Release};
    use crate::AppState;
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::sync::LazyLock;
    use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

    static POOL: LazyLock<Mutex<Pool>> = LazyLock::new(|| Mutex::new(Pool::default()));
    /// Each open window's plug-in area (an HWND), destroyed when its editor closes.
    static AREAS: LazyLock<Mutex<HashMap<String, isize>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    /// The slot's open window, if it has one.
    pub fn open_window(app: &AppHandle, slot: &str) -> Option<WebviewWindow> {
        let label = POOL.lock().open_for(slot)?;
        let window = app.get_webview_window(&label);
        if window.is_none() {
            POOL.lock().forget(&label);
        }
        window
    }

    /// A window for an editor: a ready one, told which plug-in it is for now, or a new one.
    /// The flag says it was ready, so hidden until the editor is in it.
    pub async fn take_or_make(
        app: &AppHandle,
        params: &str,
        title: &str,
    ) -> Result<(WebviewWindow, bool), String> {
        loop {
            let Some(label) = POOL.lock().take_ready() else {
                break;
            };
            if let Some(window) = app.get_webview_window(&label) {
                let _ = app.emit_to(label.as_str(), "daw:pluginHeader", params);
                let _ = window.set_title(title);
                return Ok((window, true));
            }
        }
        let label = POOL.lock().new_label();
        let window = make(app, &label, params, title, true).await?;
        Ok((window, false))
    }

    /// The editor in `window` is open: the window is the slot's now, and `area` is where the
    /// plug-in sits in it.
    pub fn editor_opened(slot: &str, window: &WebviewWindow, area: isize) {
        POOL.lock().assign(slot, window.label());
        AREAS.lock().insert(window.label().to_string(), area);
    }

    /// One window made ahead of the first editor, hidden. Called while the app starts.
    pub async fn prepare(app: AppHandle) {
        let label = POOL.lock().new_label();
        match make(&app, &label, "", "", false).await {
            Ok(_) => POOL.lock().ready(&label),
            Err(e) => log::warn!("could not make a plug-in window ahead: {e}"),
        }
    }

    /// The editor in window `label` closed: close the plug-in's GUI and its area, and keep the
    /// window hidden for the next editor if there is room. Returns whether it is kept. Runs on
    /// the main thread, where the GUI and the area were made.
    fn editor_closed(app: &AppHandle, label: &str) -> bool {
        close_gui(app, label);
        let keep = POOL.lock().release(label) == Release::Keep;
        if keep {
            if let Some(window) = app.get_webview_window(label) {
                let _ = window.hide();
                let _ = app.emit_to(label, "daw:pluginHeader", "");
            }
        }
        keep
    }

    /// The plug-in's GUI and its area in window `label`, on the main thread where both were made.
    fn close_gui(app: &AppHandle, label: &str) {
        if let Some(mut hosted) = app.state::<AppState>().plugin_editors.lock().remove(label) {
            hosted.close_editor();
        }
        if let Some(area) = AREAS.lock().remove(label) {
            crate::plugin_window_host::destroy_area(area);
        }
    }

    /// Every plug-in window, open or kept, when the main window goes: hidden windows would
    /// keep the app running with nothing on screen.
    pub fn close_all(app: &AppHandle) {
        let labels = POOL.lock().all();
        for label in labels {
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.destroy();
            }
        }
    }

    async fn make(
        app: &AppHandle,
        label: &str,
        params: &str,
        title: &str,
        visible: bool,
    ) -> Result<WebviewWindow, String> {
        // The app's own page draws the bar; the plug-in goes in an area below it
        // (plugin_window_host). `params` is already reduced to safe characters.
        let init = format!(
            "(function(){{window.__HW_PANEL__={{panel:\"pluginHeader\",params:\"{params}\"}};}})();"
        );
        let url = app
            .get_webview_window("main")
            .and_then(|w| w.url().ok())
            .map(tauri::WebviewUrl::External)
            .unwrap_or_else(|| tauri::WebviewUrl::App("index.html".into()));
        let window = tauri::WebviewWindowBuilder::new(app, label, url)
            .initialization_script(&init)
            .title(title)
            .inner_size(600.0, 400.0 + crate::plugin_window_host::HEADER_CSS_PX)
            .resizable(true)
            .always_on_top(true)
            .visible(visible)
            .build()
            .map_err(|e| format!("Failed to open editor window: {e}"))?;
        // Closing the window closes the plug-in's GUI with it, and the window is kept for the
        // next editor while there is room.
        let handle = app.clone();
        let owned = label.to_string();
        window.on_window_event(move |event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if editor_closed(&handle, &owned) {
                    api.prevent_close();
                }
            }
            tauri::WindowEvent::Destroyed => {
                close_gui(&handle, &owned);
                POOL.lock().forget(&owned);
            }
            _ => {}
        });
        Ok(window)
    }
}

#[cfg(test)]
mod tests {
    use super::{Pool, Release};

    #[test]
    fn a_slot_with_an_open_window_gets_that_window_again() {
        let mut pool = Pool::default();
        let label = pool.new_label();
        pool.assign("slot1", &label);
        assert_eq!(pool.open_for("slot1"), Some(label));
        assert_eq!(pool.open_for("slot2"), None);
    }

    #[test]
    fn a_ready_window_is_taken_once() {
        let mut pool = Pool::default();
        let label = pool.new_label();
        pool.ready(&label);
        assert_eq!(pool.take_ready(), Some(label));
        assert_eq!(pool.take_ready(), None);
    }

    #[test]
    fn a_closed_window_waits_for_the_next_editor() {
        let mut pool = Pool::default();
        let label = pool.new_label();
        pool.assign("slot1", &label);
        assert_eq!(pool.release(&label), Release::Keep);
        assert_eq!(pool.open_for("slot1"), None);
        assert_eq!(pool.take_ready(), Some(label));
    }

    #[test]
    fn only_a_few_closed_windows_are_kept() {
        let mut pool = Pool::default();
        let labels: Vec<String> = (0..super::KEEP + 1).map(|_| pool.new_label()).collect();
        for (i, label) in labels.iter().enumerate() {
            pool.assign(&format!("slot{i}"), label);
        }
        let kept = labels
            .iter()
            .filter(|l| pool.release(l) == Release::Keep)
            .count();
        assert_eq!(kept, super::KEEP);
    }

    #[test]
    fn a_window_released_twice_is_kept_once() {
        let mut pool = Pool::default();
        let label = pool.new_label();
        pool.assign("slot1", &label);
        assert_eq!(pool.release(&label), Release::Keep);
        assert_eq!(pool.release(&label), Release::Close);
        assert_eq!(pool.take_ready(), Some(label));
        assert_eq!(pool.take_ready(), None);
    }

    #[test]
    fn a_window_that_is_gone_is_forgotten() {
        let mut pool = Pool::default();
        let open = pool.new_label();
        let ready = pool.new_label();
        pool.assign("slot1", &open);
        pool.ready(&ready);
        pool.forget(&open);
        pool.forget(&ready);
        assert_eq!(pool.open_for("slot1"), None);
        assert_eq!(pool.take_ready(), None);
    }

    #[test]
    fn labels_are_new_and_inside_the_panel_permission() {
        let mut pool = Pool::default();
        let a = pool.new_label();
        let b = pool.new_label();
        assert_ne!(a, b);
        assert!(a.starts_with("panel-pluginEditor"));
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }
}
