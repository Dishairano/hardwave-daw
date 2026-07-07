//! Session logging + panic capture.
//!
//! Every run writes a session log to `~/.hardwave-daw/logs/` (last 5
//! sessions kept) so a Discord bug report can actually be debugged:
//! "audio dropped at 2:30" now comes with the backend's view of 2:30.
//! A panic hook appends the panic message + backtrace to the same file
//! and notifies the frontend so the user sees a crash banner instead of
//! a silently frozen app.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use tauri::Emitter;

const KEEP_SESSIONS: usize = 5;

/// The open session log, shared by the logger target and the panic hook.
static SESSION_LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
/// Set once Tauri is up; lets the panic hook raise a frontend banner.
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();
static SESSION_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn logs_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".hardwave-daw").join("logs"))
}

/// Tee: every log record goes to stderr (dev workflow unchanged) and,
/// when the session file opened successfully, to disk.
struct TeeWriter;

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        if let Some(f) = SESSION_LOG.get() {
            if let Ok(mut f) = f.lock() {
                let _ = f.write_all(buf);
            }
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        if let Some(f) = SESSION_LOG.get() {
            if let Ok(mut f) = f.lock() {
                let _ = f.flush();
            }
        }
        Ok(())
    }
}

/// Initialize session logging + the panic hook. Replaces the bare
/// `env_logger::init()`. Never fails: if the log dir can't be created
/// we degrade to stderr-only logging rather than refusing to start.
pub fn init(app_version: &str) {
    if let Some(dir) = logs_dir() {
        if std::fs::create_dir_all(&dir).is_ok() {
            prune_old_sessions(&dir);
            let name = format!(
                "session-{}.log",
                chrono::Local::now().format("%Y%m%d-%H%M%S")
            );
            let path = dir.join(name);
            if let Ok(f) = std::fs::File::create(&path) {
                let _ = SESSION_LOG.set(Mutex::new(f));
                let _ = SESSION_LOG_PATH.set(path);
            }
        }
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .target(env_logger::Target::Pipe(Box::new(TeeWriter)))
        .init();

    log::info!(
        "Hardwave DAW v{app_version} session start ({} {})",
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    install_panic_hook();
}

pub fn set_app_handle(handle: tauri::AppHandle) {
    let _ = APP_HANDLE.set(handle);
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current();
        let msg = format!(
            "PANIC on thread '{}': {}\nbacktrace:\n{}",
            thread.name().unwrap_or("<unnamed>"),
            info,
            backtrace
        );
        // Straight to the session file (not via `log`, which could
        // itself be the panicking component) and flush immediately —
        // the process may be about to die.
        if let Some(f) = SESSION_LOG.get() {
            if let Ok(mut f) = f.lock() {
                let _ = writeln!(f, "{msg}");
                let _ = f.flush();
            }
        }
        // Raise a frontend banner when the app survives (panic on a
        // non-main thread, e.g. audio or a worker). Best-effort.
        if let Some(app) = APP_HANDLE.get() {
            let _ = app.emit(
                "backend-panic",
                format!(
                    "A background error occurred ({}). Your project is safe — save your work and restart if audio stops. Details were written to the session log.",
                    info.location().map(|l| l.to_string()).unwrap_or_default()
                ),
            );
        }
        previous(info);
    }));
}

fn prune_old_sessions(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut sessions: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| {
                    let n = n.to_string_lossy();
                    n.starts_with("session-") && n.ends_with(".log")
                })
                .unwrap_or(false)
        })
        .collect();
    // Timestamped names sort chronologically; keep the newest N-1 so
    // the file about to be created lands within the budget.
    sessions.sort();
    let keep_existing = KEEP_SESSIONS.saturating_sub(1);
    if sessions.len() > keep_existing {
        let excess = sessions.len() - keep_existing;
        for old in sessions.into_iter().take(excess) {
            let _ = std::fs::remove_file(old);
        }
    }
}

/// Path of the current session log (for "Export diagnostics").
#[tauri::command]
pub fn diagnostics_info() -> Result<serde_json::Value, String> {
    let dir = logs_dir().ok_or("no home directory")?;
    Ok(serde_json::json!({
        "logsDir": dir.to_string_lossy(),
        "currentSessionLog": SESSION_LOG_PATH
            .get()
            .map(|p| p.to_string_lossy().to_string()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_newest_sessions() {
        let dir = std::env::temp_dir().join(format!("hw-diag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..8 {
            std::fs::write(dir.join(format!("session-2026010{}-000000.log", i)), b"x").unwrap();
        }
        std::fs::write(dir.join("unrelated.txt"), b"x").unwrap();
        prune_old_sessions(&dir);
        let logs: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("session-"))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(logs.len(), KEEP_SESSIONS - 1, "prune leaves room for the new session");
        assert!(
            logs.iter().all(|n| n.contains("2026010")),
            "kept files are session logs"
        );
        let mut sorted = logs.clone();
        sorted.sort();
        assert!(
            sorted.last().unwrap().contains("20260107"),
            "newest session survives pruning"
        );
        assert!(dir.join("unrelated.txt").exists(), "non-log files untouched");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
