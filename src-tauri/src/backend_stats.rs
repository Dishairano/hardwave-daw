//! How well the backend keeps up, for the FPS meter.
//!
//! Commands without `async` run on the main thread, the one that also
//! answers every other call from the interface and hands events to the
//! page. When one of them takes long, or the thread is busy with
//! something else, every call behind it waits: a plug-in window's meters
//! then move late and in jumps while the page itself still draws at full
//! speed. The meter showed calls taking 20 to 50 ms while the song played
//! but could not say why. This says how late the main thread is (a
//! watchdog asks it to run a no-op ten times a second) and which commands
//! took longest while they ran on it.

use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Commands quicker than this are not worth listing.
const SLOW: Duration = Duration::from_millis(2);
/// How often the watchdog checks the main thread.
const WATCH_EVERY: Duration = Duration::from_millis(100);

#[derive(Default)]
struct Window {
    lag_ms: Vec<f32>,
    commands: HashMap<String, CommandTime>,
}

#[derive(Default, Clone, Copy)]
struct CommandTime {
    count: u32,
    total_ms: f64,
    max_ms: f32,
}

fn window() -> &'static Mutex<Window> {
    static W: OnceLock<Mutex<Window>> = OnceLock::new();
    W.get_or_init(Mutex::default)
}

/// The command the main thread is in right now, and since when: what a
/// stuck main thread was doing, for the log.
fn running() -> &'static Mutex<Option<(String, Instant)>> {
    static R: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
    R.get_or_init(Mutex::default)
}

/// A command starts on the thread that runs it.
pub fn command_started(command: &str) {
    *running().lock() = Some((command.to_string(), Instant::now()));
}

/// One command finished on the thread that ran it, after `took`.
pub fn note_command(command: &str, took: Duration) {
    {
        let mut r = running().lock();
        if r.as_ref().is_some_and(|(c, _)| c == command) {
            *r = None;
        }
    }
    if took < SLOW {
        return;
    }
    let ms = took.as_secs_f64() * 1000.0;
    let mut w = window().lock();
    let entry = w.commands.entry(command.to_string()).or_default();
    entry.count += 1;
    entry.total_ms += ms;
    entry.max_ms = entry.max_ms.max(ms as f32);
}

/// What the log says while the main thread does not answer. A blocked main
/// thread freezes every window and command, and Windows waits on it during
/// a log-on or a remote-desktop reconnect ("Welcome" hangs), so it has to
/// say what it was stuck in.
fn stuck_message(stuck_for: Duration, running: Option<(&str, Duration)>) -> String {
    let doing = match running {
        Some((command, for_)) => format!("in command {command} for {:.1} s", for_.as_secs_f32()),
        None => "not in a command (a window message, a plug-in window or a posted task)".into(),
    };
    format!(
        "main thread has not answered for {:.0} s, {doing}",
        stuck_for.as_secs_f32()
    )
}

/// Start the watchdog once: ten times a second it asks the main thread to
/// run nothing and records how long that took to happen. When it takes
/// longer than two seconds it says so in the log, every 30 seconds while it
/// lasts, and once more when the thread is back.
pub fn start_watchdog(app: tauri::AppHandle) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("main-thread-watchdog".into())
        .spawn(move || loop {
            std::thread::sleep(WATCH_EVERY);
            let asked = Instant::now();
            let (tx, rx) = std::sync::mpsc::sync_channel::<Duration>(1);
            if app
                .run_on_main_thread(move || {
                    let _ = tx.send(asked.elapsed());
                })
                .is_err()
            {
                return;
            }
            // Two seconds is "stuck" and enough to show it.
            let lag = match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(lag) => lag,
                Err(_) => {
                    let mut next_note = Duration::from_secs(2);
                    let lag = loop {
                        if asked.elapsed() >= next_note {
                            let r = running().lock().clone();
                            log::warn!(
                                "{}",
                                stuck_message(
                                    asked.elapsed(),
                                    r.as_ref().map(|(c, t)| (c.as_str(), t.elapsed()))
                                )
                            );
                            next_note += Duration::from_secs(30);
                        }
                        match rx.recv_timeout(Duration::from_secs(1)) {
                            Ok(lag) => break lag,
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(_) => return,
                        }
                    };
                    log::warn!(
                        "main thread answered again after {:.1} s",
                        lag.as_secs_f32()
                    );
                    lag
                }
            };
            let mut w = window().lock();
            w.lag_ms.push(lag.as_secs_f32() * 1000.0);
            // Keep it bounded if nobody reads for a while.
            if w.lag_ms.len() > 600 {
                w.lag_ms.drain(..300);
            }
        });
    if let Err(e) = spawned {
        log::warn!("backend stats: no watchdog thread: {e}");
        STARTED.store(false, Ordering::Release);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlowCommand {
    pub command: String,
    pub count: u32,
    pub avg_ms: f32,
    pub max_ms: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStats {
    /// How late the main thread ran the watchdog's no-op, ms.
    pub main_lag_avg_ms: f32,
    pub main_lag_max_ms: f32,
    /// The commands that took longest in all (2 ms or more each), most first.
    pub slow: Vec<SlowCommand>,
}

/// What happened since the last call, then start a new window. Runs off
/// the main thread, so it answers even when that thread is busy.
#[tauri::command(async)]
pub fn get_backend_stats(app: tauri::AppHandle) -> BackendStats {
    start_watchdog(app);
    let w = std::mem::take(&mut *window().lock());
    let lag_avg = if w.lag_ms.is_empty() {
        0.0
    } else {
        w.lag_ms.iter().sum::<f32>() / w.lag_ms.len() as f32
    };
    let lag_max = w.lag_ms.iter().copied().fold(0.0, f32::max);
    let mut slow: Vec<SlowCommand> = w
        .commands
        .into_iter()
        .map(|(command, t)| SlowCommand {
            command,
            count: t.count,
            avg_ms: (t.total_ms / t.count.max(1) as f64) as f32,
            max_ms: t.max_ms,
        })
        .collect();
    slow.sort_by(|a, b| {
        (b.avg_ms * b.count as f32)
            .partial_cmp(&(a.avg_ms * a.count as f32))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    slow.truncate(3);
    BackendStats {
        main_lag_avg_ms: lag_avg,
        main_lag_max_ms: lag_max,
        slow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stuck_main_thread_says_what_it_was_in() {
        assert_eq!(
            stuck_message(
                Duration::from_secs(12),
                Some(("get_meters", Duration::from_millis(11_500)))
            ),
            "main thread has not answered for 12 s, in command get_meters for 11.5 s"
        );
        assert!(stuck_message(Duration::from_secs(2), None).contains("not in a command"));
        command_started("load_project");
        assert_eq!(
            running().lock().as_ref().map(|(c, _)| c.clone()),
            Some("load_project".to_string())
        );
        note_command("load_project", Duration::from_millis(1));
        assert!(running().lock().is_none());
    }

    #[test]
    fn quick_commands_are_not_listed_and_slow_ones_add_up() {
        let mut w = window().lock();
        *w = Window::default();
        drop(w);
        note_command("quick", Duration::from_micros(300));
        note_command("slow", Duration::from_millis(10));
        note_command("slow", Duration::from_millis(30));
        let w = window().lock();
        assert!(!w.commands.contains_key("quick"));
        let t = w.commands["slow"];
        assert_eq!(t.count, 2);
        assert!((t.total_ms - 40.0).abs() < 0.5);
        assert!((t.max_ms - 30.0).abs() < 0.5);
    }
}
