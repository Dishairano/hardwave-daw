//! What a window of ours may show.
//!
//! Our windows hold the app, and the app can do anything the DAW can:
//! open files, scan plug-ins, send things to Workspace. A page from
//! anywhere else must never end up in one of them, whether through a
//! link, a redirect, or a frame that navigates the window it sits in.
//! Every navigation is checked here: the app's own pages load, a web
//! link opens in the person's browser instead, and anything else is
//! refused.
//!
//! The command list in build.rs is the second line: even a page that
//! got in would be a remote page, and remote pages are granted nothing.

use tauri::{plugin::TauriPlugin, Runtime};
use url::Url;

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// One of the app's own pages.
    Load,
    /// A web page: the person's browser, not our window.
    OpenOutside,
    Refuse,
}

pub fn decide(url: &Url) -> Decision {
    let host = url.host_str().unwrap_or("");
    match url.scheme() {
        // The bundled app, as each platform serves it.
        "tauri" if host == "localhost" => Decision::Load,
        "http" | "https" if host == "tauri.localhost" => Decision::Load,
        // Plug-in editor windows start empty and are drawn natively.
        "about" if url.as_str() == "about:blank" => Decision::Load,
        // The development server, only in a development build.
        "http" if cfg!(debug_assertions) && host == "localhost" && url.port() == Some(5173) => {
            Decision::Load
        }
        "http" | "https" | "mailto" => Decision::OpenOutside,
        _ => Decision::Refuse,
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("window-guard")
        .on_navigation(|webview, url| match decide(url) {
            Decision::Load => true,
            Decision::OpenOutside => {
                use tauri_plugin_opener::OpenerExt;
                let _ = webview.opener().open_url(url.as_str(), None::<&str>);
                false
            }
            Decision::Refuse => {
                eprintln!("[window-guard] refused navigation to {}", url.scheme());
                false
            }
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decision {
        decide(&Url::parse(s).unwrap())
    }

    #[test]
    fn only_our_own_pages_load_in_our_windows() {
        assert_eq!(d("tauri://localhost/index.html"), Decision::Load);
        assert_eq!(d("http://tauri.localhost/"), Decision::Load);
        assert_eq!(d("https://tauri.localhost/screen"), Decision::Load);
        assert_eq!(d("about:blank"), Decision::Load);
    }

    #[test]
    fn a_web_page_goes_to_the_browser_not_the_window() {
        assert_eq!(
            d("https://hardwavestudios.com/pricing"),
            Decision::OpenOutside
        );
        assert_eq!(
            d("https://suite.hardwavestudios.com/roadmap"),
            Decision::OpenOutside
        );
        assert_eq!(
            d("https://evil.example/tauri.localhost"),
            Decision::OpenOutside
        );
        assert_eq!(
            d("https://tauri.localhost.evil.example/"),
            Decision::OpenOutside
        );
    }

    #[test]
    fn everything_else_is_refused() {
        assert_eq!(d("file:///C:/Windows/System32/calc.exe"), Decision::Refuse);
        assert_eq!(d("javascript:alert(1)"), Decision::Refuse);
        assert_eq!(d("data:text/html,<script>1</script>"), Decision::Refuse);
        assert_eq!(d("tauri://evil.example/"), Decision::Refuse);
        assert_eq!(d("ms-settings:"), Decision::Refuse);
    }

    #[test]
    fn the_dev_server_is_only_ours_in_a_development_build() {
        let expected = if cfg!(debug_assertions) {
            Decision::Load
        } else {
            Decision::OpenOutside
        };
        assert_eq!(d("http://localhost:5173/"), expected);
        assert_eq!(d("http://localhost:8080/"), Decision::OpenOutside);
    }
}
