//! A page that is not ours gets nothing.
//!
//! These call commands the way a page in a window does, through the
//! real command list and capabilities the app is built with. A command
//! from the app's own page goes through; the same command from any
//! other origin, or from a window the capabilities do not name, is
//! refused before it runs.

use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::Manager;

fn app() -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::commands::engine::list_audio_hosts
        ])
        .build(tauri::generate_context!())
        .expect("the app builds on the mock runtime")
}

fn window(
    app: &tauri::App<tauri::test::MockRuntime>,
    label: &str,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    app.get_webview_window(label).unwrap_or_else(|| {
        tauri::WebviewWindowBuilder::new(app, label, Default::default())
            .build()
            .expect("a window")
    })
}

fn call(window: &tauri::WebviewWindow<tauri::test::MockRuntime>, from: &str) -> Result<(), String> {
    get_ipc_response(
        window,
        InvokeRequest {
            cmd: "list_audio_hosts".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: from.parse().unwrap(),
            body: InvokeBody::default(),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|_| ())
    .map_err(|e| format!("{e:?}"))
}

#[test]
fn our_own_page_may_call_a_command() {
    let app = app();
    let main = window(&app, "main");
    // Where the bundled app is served from on this platform.
    let own = if cfg!(windows) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    };
    assert_eq!(call(&main, own), Ok(()));
}

#[test]
fn a_page_from_anywhere_else_may_not() {
    let app = app();
    let main = window(&app, "main");
    for origin in [
        "https://evil.example",
        "https://suite.hardwavestudios.com",
        "http://localhost:9999",
    ] {
        let refused = call(&main, origin);
        assert!(refused.is_err(), "{origin} was allowed to call a command");
    }
}

#[test]
fn a_window_the_capabilities_do_not_name_gets_nothing() {
    let app = app();
    let stranger = window(&app, "plugin-editor-7");
    assert!(call(&stranger, "tauri://localhost").is_err());
}
