//! The main window, tried again when WebView2 does not start.
//!
//! Tauri builds the main window while the app starts. On the test VM under load (19 and 20 Oct
//! 2026) WebView2 gave up after 2 to 4 minutes with 0x800705B4 ("the timeout period expired").
//! Tauri only logs that ("failed to create webview"); the app then ran on with no window at all,
//! unseen, until it was ended in Task Manager. A window that failed this way cannot be built
//! again in the same process (Tauri keeps its label taken), so the app starts itself again, a
//! few times, and after the last try says why it cannot open instead of running unseen.
//!
//! WebView2 can also never answer at all (seen with a runtime folder it cannot use): then
//! Tauri's build of the window never returns. A guard thread waits for the app's setup to be
//! reached and, when it is not, says so and ends the process.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

/// Tries in all, the first start included.
const TRIES: u32 = 3;

/// The try a start is, passed from one start to the next.
const TRY_VAR: &str = "HARDWAVE_DAW_WINDOW_TRY";

/// Wait before the next try: the computer was too busy a moment ago.
const PAUSE: Duration = Duration::from_secs(3);

/// How long the window may take before the guard gives up on it. WebView2's own timeout came
/// after 2 to 4 minutes, and that case is tried again; this is only for no answer at all.
const NO_ANSWER: Duration = Duration::from_secs(6 * 60);

static THIS_TRY: AtomicU32 = AtomicU32::new(1);

/// Set when setup is reached, whether the window opened or not.
static SETUP_REACHED: AtomicBool = AtomicBool::new(false);

/// What to do when the main window did not open.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// Start the app again as this try.
    Again(u32),
    /// Say so and quit.
    GiveUp,
}

/// Why the window did not open, for the message.
#[derive(Clone, Copy)]
enum Cause {
    /// WebView2 reported a failure on every try.
    Failed,
    /// WebView2 did not answer at all.
    NoAnswer,
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

/// Read the try count an earlier start passed on, and take it out of the environment so the
/// processes this one starts (plug-in hosts, a later restart after an update) count from 1.
/// Call at the start of `run`, before any thread of ours exists.
pub fn take_try() {
    THIS_TRY.store(
        this_try(std::env::var(TRY_VAR).ok().as_deref()),
        Ordering::Relaxed,
    );
    std::env::remove_var(TRY_VAR);
}

/// Call right before the app is built: if setup is not reached within `NO_ANSWER`, the user is
/// told and the process ends.
pub fn guard() {
    let started = Instant::now();
    let spawned = std::thread::Builder::new()
        .name("main-window-guard".into())
        .spawn(move || {
            while started.elapsed() < NO_ANSWER {
                if SETUP_REACHED.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            log::error!(
                "main window: WebView2 did not answer in {} s; telling the user and quitting",
                NO_ANSWER.as_secs()
            );
            log::logger().flush();
            tell_user(Cause::NoAnswer);
            // The window may have come while the message was up; then leave the app running.
            if SETUP_REACHED.load(Ordering::Relaxed) {
                log::info!("main window: setup was reached while the message was up");
                return;
            }
            end_now(1);
        });
    if let Err(e) = spawned {
        log::warn!("main window: no guard thread: {e}");
    }
}

/// Call first in `setup`, before anything else starts. Returns when the main window is there;
/// otherwise starts the app again or, after the last try, tells the user and ends this process.
pub fn ensure(app: &AppHandle) {
    SETUP_REACHED.store(true, Ordering::Relaxed);
    let this_try = THIS_TRY.load(Ordering::Relaxed);
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
    tell_user(Cause::Failed);
    app.cleanup_before_exit();
    std::process::exit(1);
}

/// Ends the process from the guard thread while the main thread is still stuck in WebView2:
/// nothing has started yet that needs to stop cleanly, and an ordinary exit would wait on the
/// stuck thread's libraries.
#[cfg(windows)]
fn end_now(code: u32) -> ! {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
    // SAFETY: our own process.
    unsafe {
        TerminateProcess(GetCurrentProcess(), code);
    }
    std::process::exit(code as i32);
}

#[cfg(not(windows))]
fn end_now(code: u32) -> ! {
    std::process::exit(code as i32);
}

fn message(cause: Cause) -> String {
    let what = match cause {
        Cause::Failed => format!("did not start in time ({TRIES} tries)"),
        Cause::NoAnswer => format!("did not answer for {} minutes", NO_ANSWER.as_secs() / 60),
    };
    let mut text = format!(
        "Hardwave DAW could not open its window.\n\n\
         WebView2, the part of Windows that draws Hardwave DAW, {what}. This happens when the \
         computer is very busy. Close other programs and start Hardwave DAW again."
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
fn tell_user(cause: Cause) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    };
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let text = wide(&message(cause));
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
fn tell_user(cause: Cause) {
    eprintln!("{}", message(cause));
}

#[cfg(test)]
mod tests {
    use super::{after_failed, message, this_try, Cause, Next, TRIES};

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
        let failed = message(Cause::Failed);
        assert!(failed.contains("did not start in time (3 tries)"));
        let silent = message(Cause::NoAnswer);
        assert!(silent.contains("did not answer for 6 minutes"));
        for text in [failed, silent] {
            assert!(text.contains("WebView2"));
            assert!(text.contains("Close other programs"));
            assert!(!text.contains('\u{2014}'));
        }
    }
}
