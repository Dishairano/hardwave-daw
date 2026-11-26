//! The main window, tried again when WebView2 does not start.
//!
//! On the test VM under load (19 and 20 Oct 2026, again 26 Nov 2026) WebView2 gave up on the
//! main window with 0x800705B4 ("the timeout period expired"). A window that failed this way is
//! not tried again in the same process: the app starts itself again, a few times, tells the
//! person it is doing so, and after the last try says why it cannot open instead of running
//! unseen.
//!
//! The app builds the main window itself, first thing in its setup (`ensure`), and
//! tauri.conf.json says `"create": false` for it. When Tauri builds a window from its settings
//! and that fails, Tauri panics ("Failed to setup app") before any code of ours runs: that is
//! how v0.266.46 ended on 26 Nov, with exit code 101 and no second try. (The Tauri of October
//! only logged the failure and went on without a window, which is what the tries were written
//! for.)
//!
//! WebView2 can also never answer at all (seen with a runtime folder it cannot use): then the
//! build of the window never returns. A guard thread waits for the answer and, when there is
//! none, says so and ends the process.
//!
//! To see all of this without a busy computer, start the app with
//! `HARDWAVE_DAW_SIMULATE_WEBVIEW_TIMEOUT=2` (the first 2 tries fail with WebView2's timeout
//! error, the third opens; 3 shows the give-up message) or `=hang` (WebView2 never answers).
//! The setting can only make the start fail or wait; it does nothing unless it is set.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

/// Tries in all, the first start included.
const TRIES: u32 = 3;

/// The try a start is, passed from one start to the next.
const TRY_VAR: &str = "HARDWAVE_DAW_WINDOW_TRY";

/// Makes the first tries fail the way a slow WebView2 does, for tests (see the top of the file).
const SIMULATE_VAR: &str = "HARDWAVE_DAW_SIMULATE_WEBVIEW_TIMEOUT";

/// What Tauri reports when WebView2 times out, as logged on 26 Nov 2026.
const TIMEOUT_ERROR: &str = "runtime error: failed to create webview: WebView2 error: \
     WindowsError(Error { code: HRESULT(0x800705B4), message: \"This operation returned \
     because the timeout period expired.\" })";

/// How long the window may take before the guard gives up on it. WebView2's own timeout comes
/// sooner, and that case is tried again; this is only for no answer at all.
const NO_ANSWER: Duration = Duration::from_secs(6 * 60);

static THIS_TRY: AtomicU32 = AtomicU32::new(1);

/// Set when the build of the main window came back, whether the window opened or not.
static ANSWERED: AtomicBool = AtomicBool::new(false);

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

/// What `SIMULATE_VAR` asks for.
#[derive(Debug, PartialEq, Eq)]
enum Simulate {
    /// This many tries, counted from the first, fail with WebView2's timeout error.
    Fail(u32),
    /// WebView2 never answers.
    Hang,
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

/// Wait before a later try builds its window: the computer was too busy a moment ago, and on
/// 26 Nov WebView2 gave up within seconds, so the last try waits longer.
fn pause_before(this_try: u32) -> Duration {
    match this_try {
        0 | 1 => Duration::ZERO,
        2 => Duration::from_secs(3),
        _ => Duration::from_secs(10),
    }
}

/// Anything we cannot use simulates nothing.
fn simulate(setting: Option<&str>) -> Option<Simulate> {
    let setting = setting?.trim();
    if setting.eq_ignore_ascii_case("hang") {
        return Some(Simulate::Hang);
    }
    match setting.parse::<u32>() {
        Ok(n) if n > 0 => Some(Simulate::Fail(n.min(TRIES))),
        _ => None,
    }
}

/// Build the window with `create`, unless this try is one the simulation fails. `hang` stands
/// in for a WebView2 that never answers.
fn create_or_simulated(
    this_try: u32,
    simulate: Option<Simulate>,
    hang: impl FnOnce(),
    create: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    match simulate {
        Some(Simulate::Fail(n)) if this_try <= n => Err(TIMEOUT_ERROR.to_string()),
        Some(Simulate::Hang) => {
            hang();
            Err("the simulated WebView2 did not answer".to_string())
        }
        _ => create(),
    }
}

/// The step after the build of the window came back: None when the window is there.
fn after_build(this_try: u32, built: &Result<(), String>) -> Option<Next> {
    built.as_ref().err().map(|_| after_failed(this_try))
}

/// Wait until `answered` is set. False when it was not within `limit`.
fn answered_within(limit: Duration, poll: Duration, answered: &AtomicBool) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if answered.load(Ordering::Relaxed) {
            return true;
        }
        std::thread::sleep(poll);
    }
    answered.load(Ordering::Relaxed)
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

/// Call right before the app is built: if the main window's build has not come back within
/// `NO_ANSWER`, the user is told and the process ends.
pub fn guard() {
    let spawned = std::thread::Builder::new()
        .name("main-window-guard".into())
        .spawn(move || {
            if answered_within(NO_ANSWER, Duration::from_secs(1), &ANSWERED) {
                return;
            }
            log::error!(
                "main window: WebView2 did not answer in {} s; telling the user and quitting",
                NO_ANSWER.as_secs()
            );
            log::logger().flush();
            tell_user(Cause::NoAnswer);
            // The window may have come while the message was up; then leave the app running.
            if ANSWERED.load(Ordering::Relaxed) {
                log::info!("main window: WebView2 answered while the message was up");
                return;
            }
            end_now(1);
        });
    if let Err(e) = spawned {
        log::warn!("main window: no guard thread: {e}");
    }
}

/// Call first in `setup`, before anything else starts. Builds the main window and returns when
/// it is there; otherwise starts the app again or, after the last try, tells the user and ends
/// this process.
pub fn ensure(app: &AppHandle) {
    let this_try = THIS_TRY.load(Ordering::Relaxed);
    // A later try says what is going on while it waits and builds: until the window opens
    // there is nothing else on screen.
    let notice = (this_try > 1).then(|| {
        let pause = pause_before(this_try);
        log::info!(
            "main window: try {this_try} of {TRIES}; telling the user, building the window in {} s",
            pause.as_secs()
        );
        let notice = notice::show(retry_message(this_try));
        std::thread::sleep(pause);
        notice
    });
    let simulate = simulate(std::env::var(SIMULATE_VAR).ok().as_deref());
    if let Some(simulate) = &simulate {
        log::warn!("main window: {SIMULATE_VAR} is set ({simulate:?}); this start is a test");
    }
    let built = create_or_simulated(this_try, simulate, never_answer, || build(app));
    ANSWERED.store(true, Ordering::Relaxed);
    if let Some(notice) = notice {
        notice.close();
    }
    let why = built.as_ref().err().cloned().unwrap_or_default();
    match after_build(this_try, &built) {
        None => {
            if this_try > 1 {
                log::info!("main window opened on try {this_try} of {TRIES}");
            }
        }
        Some(Next::Again(next)) => {
            log::warn!(
                "main window did not open (try {this_try} of {TRIES}): {why}; starting again"
            );
            start_again(app, next);
        }
        Some(Next::GiveUp) => {
            log::error!(
                "main window did not open in {TRIES} tries: {why}; telling the user and quitting"
            );
            give_up(app);
        }
    }
}

/// The main window from tauri.conf.json, built here so a failure comes back as an error.
fn build(app: &AppHandle) -> Result<(), String> {
    let config = app
        .config()
        .app
        .windows
        .first()
        .cloned()
        .ok_or("the app's settings name no main window")?;
    tauri::WebviewWindowBuilder::from_config(app, &config)
        .and_then(|builder| builder.build())
        .map_err(|e| e.to_string())?;
    if window_is_there(app) {
        Ok(())
    } else {
        Err("the window was made but does not answer".to_string())
    }
}

/// A window whose webview failed can still be known by its label; asking it anything fails,
/// because the runtime never kept the window.
fn window_is_there(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .is_some_and(|w| w.inner_size().is_ok())
}

/// The simulated WebView2 that never answers: the guard ends the process.
fn never_answer() {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
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

/// Shown by a later try while it waits for its window.
fn retry_message(this_try: u32) -> String {
    format!(
        "Windows is busy, and the Hardwave DAW window is taking longer than usual to open.\n\n\
         Trying again ({this_try} of {TRIES}). You do not need to do anything: this message \
         closes by itself when the window opens."
    )
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

/// A message box with no owner window, on top. Returns when it is closed.
#[cfg(windows)]
fn message_box(text: &str, icon: u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    };
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let text = wide(text);
    let title = wide("Hardwave DAW");
    // SAFETY: both strings end in a 0 and outlive the call; no owner window.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | icon | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

#[cfg(windows)]
fn tell_user(cause: Cause) {
    message_box(
        &message(cause),
        windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
    );
}

/// Elsewhere the message goes to the log and the terminal only.
#[cfg(not(windows))]
fn tell_user(cause: Cause) {
    eprintln!("{}", message(cause));
}

/// The "trying again" message: up while a later try waits for its window, without holding up
/// the main thread, and closed from the main thread when the build came back.
#[cfg(windows)]
mod notice {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostThreadMessageW, MB_ICONINFORMATION, WM_QUIT,
    };

    /// The id of the thread whose message box is up; 0 before and after.
    pub struct Notice(Arc<AtomicU32>);

    pub fn show(text: String) -> Notice {
        let thread_id = Arc::new(AtomicU32::new(0));
        let shared = Arc::clone(&thread_id);
        let spawned = std::thread::Builder::new()
            .name("main-window-notice".into())
            .spawn(move || {
                // SAFETY: no arguments, cannot fail.
                shared.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
                super::message_box(&text, MB_ICONINFORMATION);
                shared.store(0, Ordering::SeqCst);
            });
        if let Err(e) = spawned {
            log::warn!("main window: no thread for the message: {e}");
        }
        Notice(thread_id)
    }

    impl Notice {
        /// Close the box if it is still up: a message box ends when its thread is asked to
        /// quit. If that does not reach it, the box stays until OK is clicked, which is all.
        pub fn close(&self) {
            let thread_id = self.0.load(Ordering::SeqCst);
            if thread_id != 0 {
                // SAFETY: posts a message to a thread of our own process.
                unsafe {
                    PostThreadMessageW(thread_id, WM_QUIT, 0, 0);
                }
            }
        }
    }
}

/// Elsewhere the message goes to the terminal only.
#[cfg(not(windows))]
mod notice {
    pub struct Notice;

    pub fn show(text: String) -> Notice {
        eprintln!("{text}");
        Notice
    }

    impl Notice {
        pub fn close(&self) {}
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::{
        after_build, after_failed, answered_within, create_or_simulated, message, pause_before,
        retry_message, simulate, this_try, Cause, Next, Simulate, TIMEOUT_ERROR, TRIES,
    };

    /// WebView2's timeout as the window build reports it.
    fn timed_out() -> Result<(), String> {
        Err(TIMEOUT_ERROR.to_string())
    }

    /// One start: what it does about its window, with `create` as the build.
    fn start(
        this_try: u32,
        setting: Option<&str>,
        create: impl FnOnce() -> Result<(), String>,
    ) -> Option<Next> {
        let built = create_or_simulated(this_try, simulate(setting), || {}, create);
        after_build(this_try, &built)
    }

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

    /// The 26 Nov 2026 crash: Tauri built the main window from tauri.conf.json and panicked
    /// when WebView2 timed out, before the tries could run. The app has to build it itself.
    #[test]
    fn tauri_does_not_build_the_main_window_itself() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let windows = conf["app"]["windows"].as_array().expect("app.windows");
        assert_eq!(windows.len(), 1, "main_window::build builds the first one");
        let main = &windows[0];
        assert_eq!(
            main["create"],
            serde_json::Value::Bool(false),
            "with \"create\" left out Tauri builds the window and panics when WebView2 times out"
        );
        assert_eq!(
            main.get("label").and_then(|l| l.as_str()).unwrap_or("main"),
            "main"
        );
    }

    #[test]
    fn a_webview2_timeout_starts_the_app_again_instead_of_ending_it() {
        assert_eq!(start(1, None, timed_out), Some(Next::Again(2)));
        assert_eq!(start(2, None, timed_out), Some(Next::Again(3)));
        assert_eq!(start(3, None, timed_out), Some(Next::GiveUp));
    }

    #[test]
    fn a_window_that_opens_ends_the_tries() {
        for this_try in 1..=TRIES {
            assert_eq!(start(this_try, None, || Ok(())), None);
        }
    }

    #[test]
    fn a_slow_start_that_recovers_opens_on_the_third_try() {
        // Two simulated timeouts, then the real build, which works.
        let built = Cell::new(0);
        let create = || {
            built.set(built.get() + 1);
            Ok(())
        };
        assert_eq!(start(1, Some("2"), create), Some(Next::Again(2)));
        assert_eq!(start(2, Some("2"), create), Some(Next::Again(3)));
        assert_eq!(
            built.get(),
            0,
            "a simulated timeout does not build a window"
        );
        assert_eq!(start(3, Some("2"), create), None);
        assert_eq!(built.get(), 1);
    }

    #[test]
    fn the_simulated_failure_is_the_error_webview2_gave() {
        let built = create_or_simulated(1, simulate(Some("1")), || {}, || Ok(()));
        let why = built.expect_err("try 1 is simulated to fail");
        assert!(why.contains("failed to create webview"));
        assert!(why.contains("0x800705B4"));
        assert!(why.contains("timeout period expired"));
    }

    #[test]
    fn the_simulation_setting_is_read_strictly() {
        assert_eq!(simulate(None), None);
        assert_eq!(simulate(Some("")), None);
        assert_eq!(simulate(Some("0")), None);
        assert_eq!(simulate(Some("-1")), None);
        assert_eq!(simulate(Some("yes")), None);
        assert_eq!(simulate(Some("1")), Some(Simulate::Fail(1)));
        assert_eq!(simulate(Some(" 2 ")), Some(Simulate::Fail(2)));
        assert_eq!(simulate(Some("99")), Some(Simulate::Fail(TRIES)));
        assert_eq!(simulate(Some("hang")), Some(Simulate::Hang));
        assert_eq!(simulate(Some("HANG")), Some(Simulate::Hang));
    }

    #[test]
    fn without_the_setting_nothing_is_simulated() {
        let hung = Cell::new(false);
        let built = create_or_simulated(1, simulate(None), || hung.set(true), || Ok(()));
        assert_eq!(built, Ok(()));
        assert!(!hung.get());
    }

    #[test]
    fn a_simulated_hang_never_reaches_the_build() {
        let hung = Cell::new(false);
        let created = Cell::new(false);
        let _ = create_or_simulated(
            1,
            simulate(Some("hang")),
            || hung.set(true),
            || {
                created.set(true);
                Ok(())
            },
        );
        assert!(hung.get());
        assert!(!created.get());
    }

    #[test]
    fn the_guard_notices_a_webview2_that_never_answers() {
        // The build hangs: nothing ever sets the flag.
        let answered = AtomicBool::new(false);
        assert!(!answered_within(
            Duration::from_millis(60),
            Duration::from_millis(5),
            &answered
        ));
    }

    #[test]
    fn the_guard_leaves_a_slow_start_that_answers_alone() {
        let answered = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(40));
                answered.store(true, Ordering::Relaxed);
            });
            assert!(answered_within(
                Duration::from_secs(20),
                Duration::from_millis(5),
                &answered
            ));
        });
    }

    #[test]
    fn a_later_try_waits_longer_before_it_builds() {
        assert_eq!(pause_before(1), Duration::ZERO);
        assert!(pause_before(2) > Duration::ZERO);
        assert!(pause_before(3) > pause_before(2));
    }

    #[test]
    fn the_retry_message_says_what_is_happening_and_which_try() {
        let text = retry_message(2);
        assert!(text.contains("Windows is busy"));
        assert!(text.contains("taking longer than usual"));
        assert!(text.contains("Trying again (2 of 3)"));
        assert!(retry_message(3).contains("Trying again (3 of 3)"));
        assert!(!text.contains('\u{2014}'));
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
