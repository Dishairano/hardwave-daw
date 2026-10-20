//! The main window, tried again when WebView2 does not start.
//!
//! Tauri builds the main window while the app starts. On the test VM under load (19 and 20 Oct
//! 2026) WebView2 gave up after 2 to 4 minutes with 0x800705B4 ("the timeout period expired").
//! Tauri only logs that ("failed to create webview"); the app then ran on with no window at all,
//! unseen, until it was ended in Task Manager. A window that failed this way cannot be built
//! again in the same process (Tauri keeps its label taken), so the app starts itself again, a
//! few times, and after the last try says why it cannot open instead of running unseen.

use std::time::Duration;

use tauri::{AppHandle, Manager};

/// Tries in all, the first start included.
const TRIES: u32 = 3;

/// The try a start is, passed from one start to the next.
const TRY_VAR: &str = "HARDWAVE_DAW_WINDOW_TRY";

/// Wait before the next try: the computer was too busy a moment ago.
const PAUSE: Duration = Duration::from_secs(3);

/// What to do when the main window did not open.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// Start the app again as this try.
    Again(u32),
    /// Say so and quit.
    GiveUp,
}

/// Which try this start is: 1 unless an earlier start passed on a count we can use.
fn this_try(passed: Option<&str>) -> u32 {
    passed
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|n| (1..=TRIES).contains(n))
        .unwrap_or(1)
}

fn after_failed(this_try: u32) -> Next {
    if this_try < TRIES {
        Next::Again(this_try + 1)
    } else {
        Next::GiveUp
    }
}

/// Call first in `setup`, before anything else starts. Returns when the main window is there;
/// otherwise starts the app again or, after the last try, tells the user and ends this process.
pub fn ensure(app: &AppHandle) {
    let this_try = this_try(std::env::var(TRY_VAR).ok().as_deref());
    // A later restart of the app (after an update) counts from 1 again.
    std::env::remove_var(TRY_VAR);
    if window_is_there(app) {
        if this_try > 1 {
            log::info!("main window opened on try {this_try} of {TRIES}");
        }
        return;
    }
    match after_failed(this_try) {
        Next::Again(next) => {
            log::warn!(
                "main window did not open (try {this_try} of {TRIES}); starting again in {} s",
                PAUSE.as_secs()
            );
            std::thread::sleep(PAUSE);
            start_again(app, next);
        }
        Next::GiveUp => {
            log::error!("main window did not open in {TRIES} tries; telling the user and quitting");
            give_up(app);
        }
    }
}

/// A window whose webview failed is still known by its label; asking it anything fails,
/// because the runtime never kept the window.
fn window_is_there(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .is_some_and(|w| w.inner_size().is_ok())
}

fn start_again(app: &AppHandle, next: u32) -> ! {
    let env = app.env();
    let started = tauri::process::current_binary(&env).and_then(|exe| {
        std::process::Command::new(exe)
            .args(env.args_os.iter().skip(1))
            .env(TRY_VAR, next.to_string())
            .spawn()
    });
    if let Err(e) = started {
        log::error!("could not start the app again: {e}");
        give_up(app);
    }
    log::logger().flush();
    app.cleanup_before_exit();
    std::process::exit(0);
}

fn give_up(app: &AppHandle) -> ! {
    log::logger().flush();
    tell_user();
    app.cleanup_before_exit();
    std::process::exit(1);
}

fn message() -> String {
    let mut text = format!(
        "Hardwave DAW could not open its window.\n\n\
         WebView2, the part of Windows that draws Hardwave DAW, did not start in time \
         ({TRIES} tries). This happens when the computer is very busy. Close other programs \
         and start Hardwave DAW again."
    );
    if let Some(log) = crate::diagnostics::current_session_log() {
        text.push_str(&format!(
            "\n\nIf it keeps happening, send us this log:\n{}",
            log.display()
        ));
    }
    text
}

#[cfg(windows)]
fn tell_user() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    };
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let text = wide(&message());
    let title = wide("Hardwave DAW");
    // SAFETY: both strings end in a 0 and outlive the call; no owner window.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

/// Elsewhere the message goes to the log and the terminal only.
#[cfg(not(windows))]
fn tell_user() {
    eprintln!("{}", message());
}

#[cfg(test)]
mod tests {
    use super::{after_failed, this_try, Next, TRIES};

    #[test]
    fn a_first_start_is_try_one() {
        assert_eq!(this_try(None), 1);
    }

    #[test]
    fn a_start_from_an_earlier_try_carries_its_count() {
        assert_eq!(this_try(Some("2")), 2);
        assert_eq!(this_try(Some(" 3 ")), 3);
    }

    #[test]
    fn a_count_we_cannot_use_starts_over_at_one() {
        assert_eq!(this_try(Some("")), 1);
        assert_eq!(this_try(Some("x")), 1);
        assert_eq!(this_try(Some("0")), 1);
        assert_eq!(this_try(Some("99")), 1);
    }

    #[test]
    fn the_app_tries_three_times_then_gives_up() {
        assert_eq!(TRIES, 3);
        assert_eq!(after_failed(1), Next::Again(2));
        assert_eq!(after_failed(2), Next::Again(3));
        assert_eq!(after_failed(3), Next::GiveUp);
    }

    #[test]
    fn the_message_names_the_cause_and_what_to_do() {
        let text = super::message();
        assert!(text.contains("WebView2"));
        assert!(text.contains("Close other programs"));
        assert!(!text.contains('\u{2014}'));
    }
}
