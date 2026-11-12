//! The process's own priority class on Windows.
//!
//! A program started from Task Scheduler, or from a launcher that runs at
//! low priority, comes up in the "below normal" class, and every thread in
//! it with it. On the test machine the same song in that class gave about
//! 7000 dropouts where the normal class gave a handful, so the DAW lifts
//! itself back to normal at startup, before the window and its WebView
//! processes exist (a child starts in a below-normal parent's class).
//!
//! It does not go above normal, as FL Studio does. FL draws its window in
//! its own process; ours is drawn by WebView2 processes at normal
//! priority, and above normal would put this process's background work
//! (sample loading, stretch pre-rendering, plug-in scans) ahead of the
//! page, so the interface would stall under load. The threads that make
//! the sound are raised on their own instead (MMCSS, see
//! `hardwave_audio_io::thread_priority`), which is what matters for
//! dropouts.

use std::sync::atomic::{AtomicU32, Ordering};

const IDLE: u32 = 0x40;
const BELOW_NORMAL: u32 = 0x4000;
const NORMAL: u32 = 0x20;
const ABOVE_NORMAL: u32 = 0x8000;
const HIGH: u32 = 0x80;
const REALTIME: u32 = 0x100;

/// The class the process started in, and the one it runs in now; zero
/// until `raise_to_normal` has run (and always off Windows).
static STARTED_IN: AtomicU32 = AtomicU32::new(0);
static RUNS_IN: AtomicU32 = AtomicU32::new(0);

/// The class to move to from `current`, if any: only up, and only to
/// normal. A class someone set higher on purpose is left alone.
#[cfg_attr(not(windows), allow(dead_code))]
fn lift(current: u32) -> Option<u32> {
    match current {
        IDLE | BELOW_NORMAL => Some(NORMAL),
        _ => None,
    }
}

fn name(class: u32) -> &'static str {
    match class {
        IDLE => "idle",
        BELOW_NORMAL => "below normal",
        NORMAL => "normal",
        ABOVE_NORMAL => "above normal",
        HIGH => "high",
        REALTIME => "realtime",
        _ => "unknown",
    }
}

/// Lift the process to the normal class if it started below it.
#[cfg(windows)]
pub fn raise_to_normal() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetPriorityClass, SetPriorityClass,
    };
    // SAFETY: plain Win32 calls on this process's pseudo handle.
    let current = unsafe { GetPriorityClass(GetCurrentProcess()) };
    STARTED_IN.store(current, Ordering::Relaxed);
    let mut now = current;
    if let Some(target) = lift(current) {
        // SAFETY: as above.
        if unsafe { SetPriorityClass(GetCurrentProcess(), target) } != 0 {
            now = target;
        }
    }
    RUNS_IN.store(now, Ordering::Relaxed);
}

#[cfg(not(windows))]
pub fn raise_to_normal() {}

/// The class in words, for the diagnostics: "normal", or "normal (started
/// below normal)" when it was lifted.
pub fn describe() -> String {
    describe_classes(
        STARTED_IN.load(Ordering::Relaxed),
        RUNS_IN.load(Ordering::Relaxed),
    )
}

fn describe_classes(started: u32, now: u32) -> String {
    if now == 0 {
        return "not used on this platform".to_string();
    }
    if started == now {
        name(now).to_string()
    } else {
        format!("{} (started {})", name(now), name(started))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_started_low_is_lifted_to_normal_and_no_higher() {
        assert_eq!(lift(BELOW_NORMAL), Some(NORMAL));
        assert_eq!(lift(IDLE), Some(NORMAL));
        assert_eq!(lift(NORMAL), None);
        // A class set higher on purpose stays.
        assert_eq!(lift(ABOVE_NORMAL), None);
        assert_eq!(lift(HIGH), None);
    }

    #[test]
    fn the_diagnostics_say_where_it_started() {
        assert_eq!(
            describe_classes(BELOW_NORMAL, NORMAL),
            "normal (started below normal)"
        );
        assert_eq!(describe_classes(NORMAL, NORMAL), "normal");
        assert_eq!(describe_classes(0, 0), "not used on this platform");
    }
}
