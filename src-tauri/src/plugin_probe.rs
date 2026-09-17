//! Load a plug-in in a throwaway process first, so a bad one cannot take the
//! DAW with it.
//!
//! A VST3 or CLAP is someone else's C++ running inside our process: when it
//! crashes on load, the whole app disappears, usually while opening a project,
//! which looks to the user like the DAW losing their song. Scanning is safe
//! (it only reads moduleinfo.json), so the danger begins at instantiation.
//!
//! Before a plug-in is trusted, a child process loads it, instantiates it and
//! exits. If that child dies or hangs, the plug-in is refused by name instead
//! of being loaded here. It is not a full sandbox: once a plug-in has proved
//! it can load, it still runs in-process, so a crash later during playback
//! still takes the app down. Hosting plug-ins out of process permanently is a
//! bigger piece of work, and this buys the common case cheaply.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A plug-in that hangs on load is as fatal as one that crashes, and some do:
/// a licence dialog with no window, a network check with no timeout. Ten
/// seconds is long enough for a slow sampler to map its library.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// What the probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// Loaded and instantiated cleanly.
    Ok,
    /// The child died. Carries whatever the operating system said.
    Crashed { detail: String },
    /// The child was still running when the timeout expired.
    TimedOut,
    /// The probe itself could not be run. Not the plug-in's fault, so this
    /// must not blocklist anything.
    ProbeUnavailable { detail: String },
}

impl ProbeOutcome {
    /// Whether the plug-in may be loaded in this process.
    ///
    /// A probe that could not run is treated as permission: refusing every
    /// plug-in because the probe is broken would be a worse failure than the
    /// one being prevented.
    pub fn is_safe_to_load(&self) -> bool {
        matches!(
            self,
            ProbeOutcome::Ok | ProbeOutcome::ProbeUnavailable { .. }
        )
    }

    /// The sentence shown to the user. Names the plug-in and what to do,
    /// because "failed to load plugin" tells nobody anything.
    pub fn message(&self, plugin_name: &str) -> Option<String> {
        match self {
            ProbeOutcome::Ok | ProbeOutcome::ProbeUnavailable { .. } => None,
            ProbeOutcome::Crashed { detail } => Some(format!(
                "{plugin_name} crashed while loading, so it has been switched off to keep your project open. \
                 Reinstall it or update it, then re-enable it in the plug-in manager. ({detail})"
            )),
            ProbeOutcome::TimedOut => Some(format!(
                "{plugin_name} did not finish loading after {} seconds, so it has been switched off to keep \
                 the DAW responsive. It may be waiting for a licence check. Re-enable it in the plug-in manager.",
                PROBE_TIMEOUT.as_secs()
            )),
        }
    }
}

/// How a probe finished, as the supervisor sees it. Separated from the
/// process handling so the decision can be tested without a plug-in that
/// really crashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildResult {
    ExitedOk,
    ExitedWithCode(i32),
    KilledBySignal,
    StillRunning,
}

/// Turn what happened to the child into a verdict.
pub fn classify(result: ChildResult) -> ProbeOutcome {
    match result {
        ChildResult::ExitedOk => ProbeOutcome::Ok,
        // The probe child reports a refusal to load with exit code 1 and a
        // message; anything else is the plug-in taking the process down.
        ChildResult::ExitedWithCode(1) => ProbeOutcome::Crashed {
            detail: "the plug-in reported that it could not be loaded".to_string(),
        },
        ChildResult::ExitedWithCode(code) => ProbeOutcome::Crashed {
            detail: format!("the loader process exited with code {code}"),
        },
        ChildResult::KilledBySignal => ProbeOutcome::Crashed {
            detail: "the loader process was killed, which usually means the plug-in crashed"
                .to_string(),
        },
        ChildResult::StillRunning => ProbeOutcome::TimedOut,
    }
}

/// Remembers which plug-ins have proved they load, so the cost is paid once.
///
/// Keyed by path plus size and modification time: updating a plug-in changes
/// those, and an update is exactly when a plug-in that used to work can start
/// crashing.
#[derive(Debug, Default)]
pub struct ProbeCache {
    entries: HashMap<String, ProbeOutcome>,
}

impl ProbeCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn key_for(path: &Path) -> String {
        let meta = std::fs::metadata(path).ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("{}|{size}|{modified}", path.display())
    }

    pub fn get(&self, path: &Path) -> Option<&ProbeOutcome> {
        self.entries.get(&Self::key_for(path))
    }

    pub fn remember(&mut self, path: &Path, outcome: ProbeOutcome) {
        self.entries.insert(Self::key_for(path), outcome);
    }

    /// Forget one plug-in's verdict, for "try it again" in the plug-in
    /// manager after the user reinstalls it.
    pub fn forget(&mut self, path: &Path) {
        self.entries.remove(&Self::key_for(path));
    }
}

/// Verdict for one plug-in, probing it the first time and remembering the
/// answer afterwards.
///
/// The cache is process-wide because `instantiate_plugin` is called from
/// several paths (adding a plug-in, opening its editor, hydrating a project's
/// chains) and a project with the same plug-in on eight tracks must not pay
/// for eight probes.
pub fn verdict_for(path: &Path) -> ProbeOutcome {
    let cache = shared_cache();

    if let Ok(guard) = cache.lock() {
        if let Some(known) = guard.get(path) {
            return known.clone();
        }
    }
    // Probing outside the lock: it spawns a process and waits, and holding
    // the mutex across that would stall every other plug-in load.
    let outcome = probe_plugin(path);
    if let Ok(mut guard) = cache.lock() {
        guard.remember(path, outcome.clone());
    }
    outcome
}

/// Forget a plug-in's verdict so it is tried again, for the plug-in manager's
/// "try again" after a reinstall.
pub fn retry(path: &Path) {
    if let Ok(mut guard) = shared_cache().lock() {
        guard.forget(path);
    }
}

/// One cache for the process.
///
/// This was briefly a `OnceLock` declared inside each function, which gives
/// each of them its own cache: `retry` would then clear a map nothing reads,
/// and a blocked plug-in could never be un-blocked.
fn shared_cache() -> &'static std::sync::Mutex<ProbeCache> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<ProbeCache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(ProbeCache::new()))
}

/// Run the probe for one plug-in.
///
/// The child is this same executable, re-run with `--probe-plugin`: shipping
/// one binary means the probe can never be missing or a different version
/// from the app that spawned it.
pub fn probe_plugin(path: &Path) -> ProbeOutcome {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return ProbeOutcome::ProbeUnavailable {
                detail: format!("cannot find our own executable: {e}"),
            }
        }
    };
    run_probe_command(&exe, path, PROBE_TIMEOUT)
}

fn run_probe_command(exe: &Path, plugin: &Path, timeout: Duration) -> ProbeOutcome {
    use std::process::{Command, Stdio};

    let child = match Command::new(exe)
        .arg("--probe-plugin")
        .arg(plugin)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return ProbeOutcome::ProbeUnavailable {
                detail: format!("could not start the loader process: {e}"),
            }
        }
    };

    supervise(child, timeout)
}

/// Watch a running probe and decide what happened to it.
///
/// Separate from spawning so the tests can supervise real processes that
/// crash or hang, rather than only exercising the classification.
fn supervise(mut child: std::process::Child, timeout: Duration) -> ProbeOutcome {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return classify(result_from_status(status));
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    // Leaving it running would keep whatever the plug-in is
                    // waiting on alive for the rest of the session.
                    let _ = child.kill();
                    let _ = child.wait();
                    return classify(ChildResult::StillRunning);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                return ProbeOutcome::ProbeUnavailable {
                    detail: format!("lost track of the loader process: {e}"),
                }
            }
        }
    }
}

fn result_from_status(status: std::process::ExitStatus) -> ChildResult {
    match status.code() {
        Some(0) => ChildResult::ExitedOk,
        Some(code) => ChildResult::ExitedWithCode(code),
        // No exit code on Unix means a signal, which is what a segfaulting
        // plug-in looks like from out here.
        None => ChildResult::KilledBySignal,
    }
}

/// The child half: load the plug-in and exit.
///
/// Called from `main` when `--probe-plugin` is present, before any window
/// exists. A crash here is the point: it happens in this process instead of
/// the one holding the user's project.
pub fn run_probe_child(plugin_path: &str) -> i32 {
    let path = PathBuf::from(plugin_path);
    if !path.exists() {
        eprintln!("probe: {plugin_path} does not exist");
        return 1;
    }
    match hardwave_plugin_host::probe_load(&path) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("probe: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_exit_is_the_only_thing_that_counts_as_loading() {
        assert_eq!(classify(ChildResult::ExitedOk), ProbeOutcome::Ok);
        assert!(classify(ChildResult::ExitedOk).is_safe_to_load());
    }

    #[test]
    fn a_killed_loader_blocks_the_plugin_and_says_why() {
        let outcome = classify(ChildResult::KilledBySignal);
        assert!(!outcome.is_safe_to_load());
        let msg = outcome.message("Serum").expect("must explain itself");
        assert!(msg.contains("Serum"), "the user needs to know which one");
        assert!(msg.contains("crashed"), "{msg}");
    }

    #[test]
    fn a_hanging_loader_is_stopped_rather_than_waited_on_forever() {
        let outcome = classify(ChildResult::StillRunning);
        assert_eq!(outcome, ProbeOutcome::TimedOut);
        assert!(!outcome.is_safe_to_load());
        let msg = outcome.message("Kontakt").unwrap();
        assert!(msg.contains("licence"), "the usual cause is worth naming");
    }

    #[test]
    fn a_broken_probe_lets_plugins_through_rather_than_blocking_everything() {
        // If our own probe cannot run, refusing every plug-in would be a
        // bigger failure than the one being prevented.
        let outcome = ProbeOutcome::ProbeUnavailable {
            detail: "no such executable".into(),
        };
        assert!(outcome.is_safe_to_load());
        assert_eq!(outcome.message("Serum"), None, "nothing to tell the user");
    }

    #[test]
    fn the_cache_forgets_a_plugin_that_changed_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("thing.vst3");
        std::fs::write(&plugin, b"v1").unwrap();

        let mut cache = ProbeCache::new();
        cache.remember(&plugin, ProbeOutcome::Ok);
        assert_eq!(cache.get(&plugin), Some(&ProbeOutcome::Ok));

        // An update is exactly when a plug-in that used to load can start
        // crashing, so the old verdict must not carry over.
        std::fs::write(&plugin, b"v2 is much longer than v1").unwrap();
        assert_eq!(
            cache.get(&plugin),
            None,
            "a changed plug-in is probed again"
        );
    }

    #[test]
    fn forgetting_lets_a_blocked_plugin_be_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("thing.vst3");
        std::fs::write(&plugin, b"x").unwrap();

        let mut cache = ProbeCache::new();
        cache.remember(
            &plugin,
            ProbeOutcome::Crashed {
                detail: "test".into(),
            },
        );
        assert!(cache.get(&plugin).is_some());
        cache.forget(&plugin);
        assert!(
            cache.get(&plugin).is_none(),
            "reinstalling must get a retry"
        );
    }

    /// The supervisor has to survive the thing it exists for, so these drive
    /// real processes rather than only the classification.
    #[cfg(unix)]
    fn spawn(cmd: &str) -> std::process::Child {
        std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(cmd)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn")
    }

    #[cfg(unix)]
    #[test]
    fn a_process_killed_by_a_signal_is_reported_as_a_crash() {
        // SIGKILL leaves no exit code, which is exactly how a segfaulting
        // plug-in looks from the supervising side.
        let outcome = supervise(spawn("kill -9 $$"), Duration::from_secs(5));
        assert!(!outcome.is_safe_to_load(), "got {outcome:?}");
        assert!(
            matches!(outcome, ProbeOutcome::Crashed { .. }),
            "got {outcome:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_child_that_hangs_is_killed_at_the_deadline() {
        let started = std::time::Instant::now();
        let outcome = supervise(spawn("sleep 30"), Duration::from_millis(300));
        assert_eq!(outcome, ProbeOutcome::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the deadline has to actually stop it, took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn retry_clears_the_same_cache_the_verdict_reads() {
        // Each of these briefly had its own OnceLock, so retry cleared a map
        // nothing read and a blocked plug-in stayed blocked forever.
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("thing.vst3");
        std::fs::write(&plugin, b"x").unwrap();

        shared_cache()
            .lock()
            .unwrap()
            .remember(&plugin, ProbeOutcome::Crashed { detail: "t".into() });
        assert!(shared_cache().lock().unwrap().get(&plugin).is_some());

        retry(&plugin);
        assert!(
            shared_cache().lock().unwrap().get(&plugin).is_none(),
            "retry must clear the cache verdict_for consults"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_clean_child_is_allowed() {
        assert_eq!(
            supervise(spawn("exit 0"), Duration::from_secs(5)),
            ProbeOutcome::Ok
        );
    }
}
