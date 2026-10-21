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
/// a licence dialog with no window, a network check with no timeout. Hanging
/// means doing nothing: a loader that has used no processor time for this long
/// is stopped. Ten seconds is long enough for a slow sampler to map its library.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a loader that keeps working may take in all. On a busy computer a
/// healthy plug-in can need more than `PROBE_TIMEOUT` of wall time just to get
/// the processor: on the test VM at full load LoudLab was switched off every
/// time (21 Oct 2026), though it was loading all along.
pub const PROBE_LIMIT: Duration = Duration::from_secs(60);

/// What the probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// Loaded and instantiated cleanly.
    Ok,
    /// The child died. Carries whatever the operating system said.
    Crashed { detail: String },
    /// The plug-in loaded but would not start: it reported that it could
    /// not be loaded (the probe's exit code 1). Not a crash, and the
    /// message must not say it was one.
    Refused { detail: String },
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
            ProbeOutcome::Refused { detail } => Some(format!(
                "{plugin_name} could not be loaded, so it has been switched off. \
                 Reinstall it or update it, then re-enable it in the plug-in manager. ({detail})"
            )),
            ProbeOutcome::TimedOut => Some(format!(
                "{plugin_name} stopped making progress while loading, so it has been switched off to keep \
                 the DAW responsive. It may be waiting for a licence check. Re-enable it in the plug-in manager."
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
        ChildResult::ExitedWithCode(1) => ProbeOutcome::Refused {
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

/// Plug-ins tried at the same time by `warm`.
const AT_ONCE: usize = 4;

/// Try every plug-in in `paths` that has no verdict yet, a few at a time, and
/// return once all have answered.
///
/// A load on the main thread probes a plug-in it has not seen before, and the
/// window waits for that: up to a second or two normally, and on a busy
/// machine the full timeout (the freeze log on the test VM, 21 Oct 2026). The
/// callers that can (adding a plug-in, opening a project) warm the verdicts
/// here first, off the main thread, so the load finds them waiting. Never call
/// this on the main thread.
pub fn warm(paths: &[PathBuf]) {
    let todo: Vec<&PathBuf> = match shared_cache().lock() {
        Ok(guard) => {
            let mut seen = std::collections::HashSet::new();
            paths
                .iter()
                .filter(|p| guard.get(p).is_none() && seen.insert(ProbeCache::key_for(p)))
                .collect()
        }
        Err(_) => return,
    };
    for batch in todo.chunks(AT_ONCE) {
        std::thread::scope(|scope| {
            for path in batch {
                scope.spawn(move || verdict_for(path));
            }
        });
    }
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
    // A plug-in built for another architecture is not broken: this
    // process simply cannot load it, and it runs in the helper
    // instead. Probing it here would call a working plug-in a
    // crasher and hide it from the list for good.
    if !hardwave_plugin_host::binary_arch::plugin_arch(path).matches_host() {
        return ProbeOutcome::Ok;
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return ProbeOutcome::ProbeUnavailable {
                detail: format!("cannot find our own executable: {e}"),
            }
        }
    };
    run_probe_command(&exe, path, PROBE_TIMEOUT, PROBE_LIMIT)
}

fn run_probe_command(
    exe: &Path,
    plugin: &Path,
    idle_limit: Duration,
    limit: Duration,
) -> ProbeOutcome {
    use std::process::{Command, Stdio};

    let child = match Command::new(exe)
        .arg("--probe-plugin")
        .arg(plugin)
        .stdin(Stdio::null())
        // Nobody reads them: a plug-in that logs more than a pipe holds
        // would block, time out, and be refused for being "hung".
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return ProbeOutcome::ProbeUnavailable {
                detail: format!("could not start the loader process: {e}"),
            }
        }
    };

    supervise(child, idle_limit, limit)
}

/// Whether a loader still running is given up on: when it has done no work for
/// `idle_limit`, or has run for `limit` in all.
fn give_up(ran: Duration, idle: Duration, idle_limit: Duration, limit: Duration) -> bool {
    idle >= idle_limit || ran >= limit
}

/// Processor time the child has used so far, in units that only need to grow
/// while it works. `None` where it cannot be read; then every moment counts as
/// idle and the idle limit is plain wall time, as before.
fn work_done(child: &std::process::Child) -> Option<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::GetProcessTimes;
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: the child's own handle, alive while `child` is; four out-parameters we own.
        let ok = unsafe {
            GetProcessTimes(
                child.as_raw_handle() as _,
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        } != 0;
        let ticks = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        ok.then(|| ticks(kernel) + ticks(user))
    }
    #[cfg(target_os = "linux")]
    {
        // utime and stime, fields 14 and 15, counted after the ")" that ends the name.
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.id())).ok()?;
        let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
        let utime: u64 = fields.get(11)?.parse().ok()?;
        let stime: u64 = fields.get(12)?.parse().ok()?;
        Some(utime + stime)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = child;
        None
    }
}

/// Watch a running probe and decide what happened to it.
///
/// Separate from spawning so the tests can supervise real processes that
/// crash or hang, rather than only exercising the classification.
fn supervise(
    mut child: std::process::Child,
    idle_limit: Duration,
    limit: Duration,
) -> ProbeOutcome {
    let started = std::time::Instant::now();
    let mut last_work = work_done(&child);
    let mut last_progress = started;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return classify(result_from_status(status));
            }
            Ok(None) => {
                let work = work_done(&child);
                if work.is_some() && work > last_work {
                    last_work = work;
                    last_progress = std::time::Instant::now();
                }
                if give_up(
                    started.elapsed(),
                    last_progress.elapsed(),
                    idle_limit,
                    limit,
                ) {
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
    fn a_plug_in_that_refuses_to_load_is_not_called_a_crash() {
        // Exit code 1 is the probe saying the plug-in would not start (a
        // CLAP factory that returned null, say). Calling that a crash sent
        // people looking for a crash that never happened.
        let outcome = classify(ChildResult::ExitedWithCode(1));
        assert!(
            matches!(outcome, ProbeOutcome::Refused { .. }),
            "got {outcome:?}"
        );
        assert!(!outcome.is_safe_to_load());
        let text = outcome.message("WideBoi").unwrap();
        assert!(
            text.contains("could not be loaded") && !text.contains("crashed"),
            "{text}"
        );
        // Any other exit code is still the plug-in taking the process down.
        assert!(matches!(
            classify(ChildResult::ExitedWithCode(-1073741819)),
            ProbeOutcome::Crashed { .. }
        ));
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
        let outcome = supervise(
            spawn("kill -9 $$"),
            Duration::from_secs(5),
            Duration::from_secs(5),
        );
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
        let outcome = supervise(
            spawn("sleep 30"),
            Duration::from_millis(300),
            Duration::from_secs(5),
        );
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

    /// A busy loader is not idle: it is given up on only at the overall limit.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_loader_that_keeps_working_gets_more_time() {
        let started = std::time::Instant::now();
        let outcome = supervise(
            spawn("while :; do :; done"),
            Duration::from_millis(300),
            Duration::from_millis(1500),
        );
        assert_eq!(outcome, ProbeOutcome::TimedOut);
        assert!(
            started.elapsed() >= Duration::from_millis(1400),
            "stopped after {:?}, at the idle limit instead of the overall one",
            started.elapsed()
        );
    }

    #[test]
    fn a_loader_is_given_up_on_when_idle_or_at_the_limit() {
        use super::give_up;
        let s = Duration::from_secs;
        assert!(!give_up(s(5), s(2), s(10), s(60)));
        assert!(give_up(s(12), s(10), s(10), s(60)));
        assert!(!give_up(s(40), s(1), s(10), s(60)));
        assert!(give_up(s(60), s(0), s(10), s(60)));
    }

    #[cfg(unix)]
    #[test]
    fn a_clean_child_is_allowed() {
        assert_eq!(
            supervise(
                spawn("exit 0"),
                Duration::from_secs(5),
                Duration::from_secs(5)
            ),
            ProbeOutcome::Ok
        );
    }
}
