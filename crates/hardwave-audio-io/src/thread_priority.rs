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
//! MMCSS refuses. Some ASIO drivers register their own callback thread
//! with MMCSS before they call us; that thread is left exactly as the
//! driver set it, since time-critical (15) sits below the real-time
//! range MMCSS gives (around 26 on the test machine) and would lower it.
//! It happens once per thread, on the thread's first
//! block: later blocks only read a flag the stream closure keeps. The
//! registration is handed back when the thread ends; MMCSS has a limit
//! on registered threads, and a stream is opened again on every device
//! or buffer change.
//!
//! On other platforms this does nothing (macOS joins the device's
//! workgroup in `coreaudio_workgroup` instead).
//!
//! Every thread raised here is also listed by its system id
//! ([`is_audio_thread`]), so tools that stop threads to look at them (the
//! freeze log) can leave the audio threads alone, whatever priority they
//! ended up at and whoever created them (an ASIO driver makes its own).
//!
//! The other way round, [`lower_current_thread`] puts background work
//! (stretch and warp bakes) below normal priority, so it never competes
//! with the audio threads or the window.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

/// What raising a thread came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityOutcome {
    /// The thread has not run its first block yet.
    Pending,
    /// Registered with MMCSS as "Pro Audio" at high priority.
    Mmcss,
    /// The thread was already registered with MMCSS by its owner (an
    /// ASIO driver that does this itself); left untouched.
    AlreadyRegistered,
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
            PriorityOutcome::AlreadyRegistered => (5, 0),
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
            5 => PriorityOutcome::AlreadyRegistered,
            _ => PriorityOutcome::Pending,
        }
    }

    /// In words, for the diagnostics and the log.
    pub fn describe(self) -> String {
        match self {
            PriorityOutcome::Pending => "not started".to_string(),
            PriorityOutcome::Mmcss => "mmcss Pro Audio ok".to_string(),
            PriorityOutcome::AlreadyRegistered => "driver mmcss (already registered)".to_string(),
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

/// What to do when MMCSS refuses to register a thread.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnRefusal {
    /// Someone else already registered this thread: leave its priority
    /// alone.
    LeaveAlone,
    /// A real failure (no MMCSS service, bad task name): raise the
    /// thread to time-critical ourselves.
    TimeCritical,
}

/// `ERROR_THREAD_ALREADY_IN_TASK`: the thread is already in an MMCSS task.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const ERROR_THREAD_ALREADY_IN_TASK: u32 = 1552;

/// Decide from the error MMCSS gave (as the HRESULT the Windows bindings
/// return, or the bare Win32 code) whether the thread is already
/// registered by its owner or registration really failed.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn on_refusal(code: u32) -> OnRefusal {
    // HRESULT_FROM_WIN32 puts a Win32 code in the low 16 bits under
    // facility 7 (FACILITY_WIN32) with the failure bit set.
    let win32 = if code & 0xffff_0000 == 0x8007_0000 {
        code & 0xffff
    } else {
        code
    };
    if win32 == ERROR_THREAD_ALREADY_IN_TASK {
        OnRefusal::LeaveAlone
    } else {
        OnRefusal::TimeCritical
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
        if self.outcome.get().is_some() {
            forget_audio_thread(platform::current_thread_id());
        }
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
            remember_audio_thread(platform::current_thread_id());
            t.outcome.set(Some(outcome));
            t.handle.set(handle);
            t.revert.set(Some(revert));
            outcome
        })
        // The thread is shutting down: nothing worth raising any more.
        .unwrap_or(PriorityOutcome::Pending)
}

/// Most audio threads listed at once: an output, an input, the exclusive
/// render thread and the workers, with a stream opened again on every
/// device or buffer change while an old one winds down: far more room than
/// that needs.
const MAX_AUDIO_THREADS: usize = 256;

/// A fixed list of thread ids, zero for a free entry. Lock-free and
/// allocation-free, since the audio threads add themselves.
struct ThreadList<const N: usize> {
    ids: [AtomicU64; N],
    /// Threads that found the list full and are not in it.
    missed: AtomicU64,
}

impl<const N: usize> ThreadList<N> {
    const fn new() -> Self {
        Self {
            ids: [const { AtomicU64::new(0) }; N],
            missed: AtomicU64::new(0),
        }
    }

    /// Add `id`. False when the list is full; that is counted, so the
    /// list's users know it is not complete.
    fn remember(&self, id: u64) -> bool {
        if id == 0 || self.contains(id) {
            return true;
        }
        for entry in &self.ids {
            if entry
                .compare_exchange(0, id, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
        self.missed.fetch_add(1, Ordering::AcqRel);
        false
    }

    fn forget(&self, id: u64) {
        if id == 0 {
            return;
        }
        for entry in &self.ids {
            let _ = entry.compare_exchange(id, 0, Ordering::AcqRel, Ordering::Relaxed);
        }
    }

    fn contains(&self, id: u64) -> bool {
        id != 0 && self.ids.iter().any(|e| e.load(Ordering::Acquire) == id)
    }
}

/// The system ids of the threads raised here.
static AUDIO_THREADS: ThreadList<MAX_AUDIO_THREADS> = ThreadList::new();

fn remember_audio_thread(id: u64) {
    AUDIO_THREADS.remember(id);
}

fn forget_audio_thread(id: u64) {
    AUDIO_THREADS.forget(id);
}

/// Whether the thread with this system id (`GetCurrentThreadId` on
/// Windows) renders audio: it was raised by [`raise_current_thread`] and
/// has not ended. Always false where threads have no id here.
pub fn is_audio_thread(id: u64) -> bool {
    AUDIO_THREADS.contains(id)
}

/// How many audio threads found the list full, since the app started: when
/// not zero, [`is_audio_thread`] can say no to an audio thread, and a tool
/// that stops threads must stop none.
pub fn audio_threads_missed() -> u64 {
    AUDIO_THREADS.missed.load(Ordering::Acquire)
}

/// Write a log line when more audio threads found the list full since the
/// last call. For the health poll: the audio thread itself does not format.
pub fn log_missed_audio_threads() {
    static LOGGED: AtomicU64 = AtomicU64::new(0);
    let missed = audio_threads_missed();
    if LOGGED.swap(missed, Ordering::AcqRel) != missed {
        log::warn!(
            "{missed} audio thread(s) found the audio thread list full ({MAX_AUDIO_THREADS}); \
             the freeze log stops no other threads from now on"
        );
    }
}

/// Put the calling thread below normal priority, for work that may take
/// its time: it then only gets the processor when the audio threads and
/// the window do not want it. True when the system took it.
///
/// Windows: below normal. Linux: a nice of 10 for this thread. macOS: the
/// utility quality of service. Elsewhere nothing.
pub fn lower_current_thread() -> bool {
    platform::lower()
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{on_refusal, OnRefusal, PriorityOutcome};
    use windows::core::w;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Threading::{
        AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, AvSetMmThreadPriority,
        GetCurrentThread, GetCurrentThreadId, SetThreadPriority, AVRT_PRIORITY_HIGH,
        THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_TIME_CRITICAL,
    };

    pub(super) fn current_thread_id() -> u64 {
        // SAFETY: a plain call with no arguments.
        unsafe { GetCurrentThreadId() as u64 }
    }

    pub(super) fn lower() -> bool {
        // SAFETY: GetCurrentThread is a pseudo handle valid on this thread.
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) }.is_ok()
    }

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
        if on_refusal(mmcss_error) == OnRefusal::LeaveAlone {
            // An ASIO driver registered its own thread before calling us;
            // its MMCSS priority is above anything we could set.
            return (PriorityOutcome::AlreadyRegistered, 0);
        }
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

    /// Nothing outside Windows asks for a thread by its id.
    pub(super) fn current_thread_id() -> u64 {
        0
    }

    #[cfg(target_os = "linux")]
    pub(super) fn lower() -> bool {
        // SAFETY: plain calls; on Linux a nice set by thread id applies to
        // that thread alone.
        unsafe {
            let tid = libc::gettid();
            libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, 10) == 0
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn lower() -> bool {
        // SAFETY: a plain call on the calling thread.
        unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0) == 0 }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(super) fn lower() -> bool {
        false
    }
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
            PriorityOutcome::AlreadyRegistered,
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
        assert_eq!(
            PriorityOutcome::AlreadyRegistered.describe(),
            "driver mmcss (already registered)"
        );
    }

    #[test]
    fn a_thread_the_driver_already_registered_is_left_alone() {
        // What FL Studio ASIO's callback thread gave on the test machine.
        assert_eq!(on_refusal(0x8007_0610), OnRefusal::LeaveAlone);
        // The same code bare, as GetLastError gives it.
        assert_eq!(on_refusal(1552), OnRefusal::LeaveAlone);
    }

    #[test]
    fn a_real_mmcss_failure_still_falls_back_to_time_critical() {
        for code in [
            0x8007_0005, // access denied
            0x8007_060e, // ERROR_INVALID_TASK_NAME
            0x8007_060f, // ERROR_INVALID_TASK_INDEX
            0x8007_0426, // ERROR_SERVICE_NOT_ACTIVE (MMCSS not running)
            0x8007_0002, // the task is not in the registry
            0x8000_0610, // 0x610 under another facility is not this error
            0x0000_0610 | 0x0001_0000,
            0,
        ] {
            assert_eq!(on_refusal(code), OnRefusal::TimeCritical, "{code:08x}");
        }
    }

    #[test]
    fn an_audio_thread_is_listed_until_it_ends() {
        // System ids are faked here: the list works on whatever id the
        // platform gives, and only Windows gives one.
        let id = 0x5eed_0001;
        assert!(!is_audio_thread(id));
        remember_audio_thread(id);
        remember_audio_thread(id);
        assert!(is_audio_thread(id));
        forget_audio_thread(id);
        assert!(!is_audio_thread(id), "listed once, so gone once forgotten");
        // No id: never listed.
        remember_audio_thread(0);
        assert!(!is_audio_thread(0));
    }

    /// A full list says so instead of quietly leaving a thread out.
    #[test]
    fn a_full_list_counts_the_threads_it_could_not_take() {
        let list: ThreadList<4> = ThreadList::new();
        for id in 1..=4 {
            assert!(list.remember(id));
        }
        assert_eq!(list.missed.load(Ordering::Relaxed), 0);
        assert!(list.remember(3), "one already listed is no miss");
        assert!(!list.remember(5));
        assert!(!list.contains(5));
        assert_eq!(list.missed.load(Ordering::Relaxed), 1);
        list.forget(2);
        assert!(list.remember(5), "room again once a thread ended");
        assert!(list.contains(5));
    }

    #[test]
    fn a_raised_thread_is_an_audio_thread_until_it_ends() {
        let id = std::thread::spawn(|| {
            raise_current_thread();
            let id = platform::current_thread_id();
            assert_eq!(is_audio_thread(id), id != 0);
            id
        })
        .join()
        .unwrap();
        assert!(!is_audio_thread(id));
    }

    #[test]
    fn background_work_can_be_put_below_normal() {
        std::thread::spawn(|| {
            let lowered = lower_current_thread();
            if cfg!(any(target_os = "linux", target_os = "windows")) {
                assert!(lowered);
            }
            #[cfg(target_os = "linux")]
            {
                // SAFETY: plain calls about this thread.
                let nice =
                    unsafe { libc::getpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t) };
                assert!(nice >= 10, "nice {nice}");
            }
        })
        .join()
        .unwrap();
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
