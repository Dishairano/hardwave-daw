//! Letting go of the audio device while another program is in front.
//!
//! An idle Hardwave DAW kept its audio output open, and FL Studio, set to FL
//! Studio ASIO on the same sound card, hung for minutes when it reset its
//! device (on loading a plug-in, on opening a menu): reproduced on the test
//! VM on 22 Oct 2026, gone as soon as the DAW held no device. Like FL's own
//! "Auto close device", the DAW now closes the device when the song is
//! stopped and another program has been in front for a few seconds, and opens
//! it again when one of its own windows comes forward or something asks to
//! play.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hardwave_engine::DawEngine;
use parking_lot::Mutex;

/// The setting, on unless switched off in Audio settings.
static ENABLED: AtomicBool = AtomicBool::new(true);
/// True while the device is closed because of this, so it is this that
/// opens it again and not a device the person closed themselves.
static RELEASED: AtomicBool = AtomicBool::new(false);

/// How long another program has to be in front before the device goes:
/// clicking past the DAW for a moment should not stop its audio.
const GRACE: Duration = Duration::from_secs(3);

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// What to do with the device, from what is going on.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Keep,
    Release,
    Reopen,
}

fn decide(
    enabled: bool,
    ours_in_front: bool,
    busy: bool,
    background_for: Duration,
    released: bool,
) -> Step {
    if released && (!enabled || ours_in_front || busy) {
        Step::Reopen
    } else if enabled && !released && !ours_in_front && !busy && background_for >= GRACE {
        Step::Release
    } else {
        Step::Keep
    }
}

/// Open the device again if this module closed it. Called before playing
/// or recording, so pressing play from a shortcut while another program is
/// in front still plays.
pub fn reopen_if_released(engine: &mut DawEngine) {
    if RELEASED.load(Ordering::Relaxed) {
        match engine.start() {
            Ok(()) => {
                RELEASED.store(false, Ordering::Relaxed);
                log::info!("audio device opened again");
            }
            Err(e) => log::warn!("could not open the audio device again: {e}"),
        }
    }
}

/// Watch which program is in front, twice a second, for the life of the app.
pub fn spawn(engine: Arc<Mutex<DawEngine>>) {
    let (playing, recording) = {
        let e = engine.lock();
        (
            Arc::clone(&e.transport.playing),
            Arc::clone(&e.transport.recording),
        )
    };
    let _ = std::thread::Builder::new()
        .name("audio-focus".into())
        .spawn(move || {
            let mut background_since: Option<Instant> = None;
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let ours = ours_in_front();
                let busy = playing.load(Ordering::Relaxed) || recording.load(Ordering::Relaxed);
                if ours || busy {
                    background_since = None;
                } else if background_since.is_none() {
                    background_since = Some(Instant::now());
                }
                let background_for = background_since.map(|t| t.elapsed()).unwrap_or_default();
                match decide(
                    enabled(),
                    ours,
                    busy,
                    background_for,
                    RELEASED.load(Ordering::Relaxed),
                ) {
                    Step::Keep => {}
                    Step::Release => {
                        // Never wait on the engine here: the next round will do.
                        let Some(mut e) = engine.try_lock_for(Duration::from_millis(50)) else {
                            continue;
                        };
                        if e.is_running() {
                            e.stop();
                            RELEASED.store(true, Ordering::Relaxed);
                            log::info!("audio device closed while another program is in front");
                        }
                    }
                    Step::Reopen => {
                        if let Some(mut e) = engine.try_lock_for(Duration::from_millis(50)) {
                            reopen_if_released(&mut e);
                        }
                    }
                }
            }
        });
}

/// Whether the window in front belongs to this process: the main window, a
/// detached panel or a plug-in window all do.
#[cfg(windows)]
fn ours_in_front() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };
    // SAFETY: plain queries about the foreground window; a null window (none
    // in front, or a desktop switch) counts as ours, so nothing is closed.
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return true;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, &mut pid);
        pid == std::process::id()
    }
}

/// Elsewhere the device is never let go.
#[cfg(not(windows))]
fn ours_in_front() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{decide, Step, GRACE};
    use std::time::Duration;

    #[test]
    fn the_device_goes_once_another_program_has_been_in_front_a_while() {
        assert_eq!(decide(true, false, false, GRACE, false), Step::Release);
        assert_eq!(
            decide(true, false, false, Duration::from_secs(1), false),
            Step::Keep,
            "a quick click elsewhere"
        );
    }

    #[test]
    fn a_playing_or_recording_song_keeps_it() {
        assert_eq!(decide(true, false, true, GRACE * 10, false), Step::Keep);
    }

    #[test]
    fn it_comes_back_when_the_daw_is_in_front_or_plays() {
        assert_eq!(
            decide(true, true, false, Duration::ZERO, true),
            Step::Reopen
        );
        assert_eq!(
            decide(true, false, true, Duration::ZERO, true),
            Step::Reopen
        );
    }

    #[test]
    fn switched_off_it_keeps_the_device_and_gives_back_one_it_closed() {
        assert_eq!(decide(false, false, false, GRACE * 10, false), Step::Keep);
        assert_eq!(decide(false, false, false, GRACE * 10, true), Step::Reopen);
    }
}
