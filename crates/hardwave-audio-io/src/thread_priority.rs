//! Real-time priority for the threads that make the sound.
//!
//! On Windows the thread that fills the sound card's buffer has to win
//! against everything else on the machine, or a block comes late and the
//! producer hears a click. cpal 0.15 tries to raise its WASAPI thread but
//! passes the thread's id where Windows wants a handle, so the call fails
//! and the thread stays at normal priority: measured on the test machine,
//! the same song gave about 600 dropouts a minute that way and one at
//! time-critical priority.
//!
//! So every thread that renders audio registers itself with the
//! Multimedia Class Scheduler (MMCSS) as "Pro Audio" at high priority,
//! the way other DAWs do, and falls back to time-critical priority when
//! MMCSS refuses. It happens once per thread, on the thread's first
//! block: later blocks only read a flag the stream closure keeps. The
//! registration is handed back when the thread ends; MMCSS has a limit
//! on registered threads, and a stream is opened again on every device
//! or buffer change.
//!
//! On other platforms this does nothing (macOS joins the device's
//! workgroup in `coreaudio_workgroup` instead).

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

/// What raising a thread came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityOutcome {
    /// The thread has not run its first block yet.
    Pending,
    /// Registered with MMCSS as "Pro Audio" at high priority.
    Mmcss,
    /// MMCSS refused (its error code is kept); the thread runs at
    /// time-critical priority instead.
    TimeCritical(u32),
    /// Neither worked; the error code of the last try.
    Failed(u32),
    /// Nothing to do on this platform.
    NotApplicable,
}

impl PriorityOutcome {
    fn encode(self) -> u64 {
        let (kind, code) = match self {
            PriorityOutcome::Pending => (0u64, 0u32),
            PriorityOutcome::Mmcss => (1, 0),
            PriorityOutcome::TimeCritical(code) => (2, code),
            PriorityOutcome::Failed(code) => (3, code),
            PriorityOutcome::NotApplicable => (4, 0),
        };
        (kind << 32) | code as u64
    }

    fn decode(bits: u64) -> Self {
        let code = bits as u32;
        match bits >> 32 {
            1 => PriorityOutcome::Mmcss,
            2 => PriorityOutcome::TimeCritical(code),
            3 => PriorityOutcome::Failed(code),
            4 => PriorityOutcome::NotApplicable,
            _ => PriorityOutcome::Pending,
        }
    }

    /// In words, for the diagnostics and the log.
    pub fn describe(self) -> String {
        match self {
            PriorityOutcome::Pending => "not started".to_string(),
            PriorityOutcome::Mmcss => "mmcss Pro Audio ok".to_string(),
            PriorityOutcome::TimeCritical(code) => {
                format!("fallback time critical (mmcss error 0x{code:08x})")
            }
            PriorityOutcome::Failed(code) => format!("failed 0x{code:08x}"),
            PriorityOutcome::NotApplicable => "not used on this platform".to_string(),
        }
    }
}

/// Where a stream's audio thread leaves its outcome, for the engine to
/// read from another thread. Storing is one atomic write.
#[derive(Debug, Default)]
pub struct PriorityStatus(AtomicU64);

impl PriorityStatus {
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    pub fn get(&self) -> PriorityOutcome {
        PriorityOutcome::decode(self.0.load(Ordering::Relaxed))
    }

    pub fn set(&self, outcome: PriorityOutcome) {
        self.0.store(outcome.encode(), Ordering::Relaxed);
    }
}

/// This thread's registration: what raising it came to, and the MMCSS
/// handle to give back when the thread ends.
struct ThreadState {
    outcome: Cell<Option<PriorityOutcome>>,
    handle: Cell<usize>,
    revert: Cell<Option<fn(usize)>>,
}

impl Drop for ThreadState {
    fn drop(&mut self) {
        let handle = self.handle.get();
        if handle != 0 {
            if let Some(revert) = self.revert.get() {
                revert(handle);
            }
        }
    }
}

thread_local! {
    static THREAD: ThreadState = const {
        ThreadState {
            outcome: Cell::new(None),
            handle: Cell::new(0),
            revert: Cell::new(None),
        }
    };
}

/// Raise the calling thread to real-time priority, once per thread.
///
/// The first call on a thread does the work (a few system calls) and
/// returns what it came to; every later call on that thread returns the
/// same outcome without asking the system again. Called on an audio
/// thread's first block; a stream keeps its own flag so later blocks do
/// not call this at all.
pub fn raise_current_thread() -> PriorityOutcome {
    raise_once(platform::raise, platform::revert)
}

/// The once-per-thread part, with the system calls passed in so it can
/// be tested anywhere. `raise` returns the outcome and the handle to give
/// back (zero for none); `revert` gives it back when the thread ends.
fn raise_once(
    raise: impl FnOnce() -> (PriorityOutcome, usize),
    revert: fn(usize),
) -> PriorityOutcome {
    THREAD
        .try_with(|t| {
            if let Some(done) = t.outcome.get() {
                return done;
            }
            let (outcome, handle) = raise();
            t.outcome.set(Some(outcome));
            t.handle.set(handle);
            t.revert.set(Some(revert));
            outcome
        })
        // The thread is shutting down: nothing worth raising any more.
        .unwrap_or(PriorityOutcome::Pending)
}

#[cfg(target_os = "windows")]
mod platform {
    use super::PriorityOutcome;
    use windows::core::w;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Threading::{
        AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, AvSetMmThreadPriority,
        GetCurrentThread, SetThreadPriority, AVRT_PRIORITY_HIGH, THREAD_PRIORITY_TIME_CRITICAL,
    };

    pub(super) fn raise() -> (PriorityOutcome, usize) {
        let mut task_index = 0u32;
        // SAFETY: plain Win32 calls on the current thread; the task name
        // is a static wide string and `task_index` outlives the call.
        let mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index) };
        let mmcss_error = match mmcss {
            Ok(handle) => {
                // Registered: "Pro Audio" already runs in the real-time
                // range, high asks for the top of it. A refusal here
                // still leaves the thread registered, so it counts.
                // SAFETY: the handle was just returned for this thread.
                let _ = unsafe { AvSetMmThreadPriority(handle, AVRT_PRIORITY_HIGH) };
                return (PriorityOutcome::Mmcss, handle.0 as usize);
            }
            Err(e) => e.code().0 as u32,
        };
        // SAFETY: GetCurrentThread is a pseudo handle valid on this thread.
        match unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) } {
            Ok(()) => (PriorityOutcome::TimeCritical(mmcss_error), 0),
            Err(e) => (PriorityOutcome::Failed(e.code().0 as u32), 0),
        }
    }

    pub(super) fn revert(handle: usize) {
        // SAFETY: runs on the thread that registered, as it ends; the
        // handle came from AvSetMmThreadCharacteristicsW on that thread.
        let _ = unsafe { AvRevertMmThreadCharacteristics(HANDLE(handle as *mut _)) };
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::PriorityOutcome;

    pub(super) fn raise() -> (PriorityOutcome, usize) {
        (PriorityOutcome::NotApplicable, 0)
    }

    pub(super) fn revert(_handle: usize) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_thread_is_raised_once_however_many_blocks_ask() {
        std::thread::spawn(|| {
            let calls = Cell::new(0);
            let raise = || {
                calls.set(calls.get() + 1);
                (PriorityOutcome::TimeCritical(0x8007_0005), 0)
            };
            assert_eq!(
                raise_once(raise, |_| {}),
                PriorityOutcome::TimeCritical(0x8007_0005)
            );
            // Later calls answer from the thread's own record.
            for _ in 0..3 {
                let again = raise_once(
                    || {
                        calls.set(calls.get() + 1);
                        (PriorityOutcome::Failed(1), 0)
                    },
                    |_| {},
                );
                assert_eq!(again, PriorityOutcome::TimeCritical(0x8007_0005));
            }
            assert_eq!(calls.get(), 1);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn every_new_thread_is_raised_for_itself() {
        static RAISED: AtomicUsize = AtomicUsize::new(0);
        for _ in 0..3 {
            std::thread::spawn(|| {
                raise_once(
                    || {
                        RAISED.fetch_add(1, Ordering::SeqCst);
                        (PriorityOutcome::Mmcss, 0)
                    },
                    |_| {},
                );
                raise_once(|| unreachable!("raised twice"), |_| {});
            })
            .join()
            .unwrap();
        }
        assert_eq!(RAISED.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn the_registration_is_given_back_when_the_thread_ends() {
        static GIVEN_BACK: AtomicUsize = AtomicUsize::new(0);
        fn revert(handle: usize) {
            GIVEN_BACK.store(handle, Ordering::SeqCst);
        }
        std::thread::spawn(|| {
            raise_once(|| (PriorityOutcome::Mmcss, 42), revert);
            assert_eq!(GIVEN_BACK.load(Ordering::SeqCst), 0, "not while it runs");
        })
        .join()
        .unwrap();
        assert_eq!(GIVEN_BACK.load(Ordering::SeqCst), 42);
    }

    #[test]
    fn a_status_reads_back_what_was_stored() {
        let status = PriorityStatus::new();
        assert_eq!(status.get(), PriorityOutcome::Pending);
        for outcome in [
            PriorityOutcome::Mmcss,
            PriorityOutcome::TimeCritical(0x8007_0005),
            PriorityOutcome::Failed(0xffff_ffff),
            PriorityOutcome::NotApplicable,
            PriorityOutcome::Pending,
        ] {
            status.set(outcome);
            assert_eq!(status.get(), outcome);
        }
    }

    #[test]
    fn the_outcome_reads_the_way_the_test_machine_checks_it() {
        assert_eq!(PriorityOutcome::Mmcss.describe(), "mmcss Pro Audio ok");
        assert_eq!(
            PriorityOutcome::TimeCritical(0x8007_0005).describe(),
            "fallback time critical (mmcss error 0x80070005)"
        );
        assert_eq!(PriorityOutcome::Failed(5).describe(), "failed 0x00000005");
    }

    #[test]
    fn here_it_does_nothing_off_windows() {
        if cfg!(not(target_os = "windows")) {
            std::thread::spawn(|| {
                assert_eq!(raise_current_thread(), PriorityOutcome::NotApplicable);
            })
            .join()
            .unwrap();
        }
    }
}
