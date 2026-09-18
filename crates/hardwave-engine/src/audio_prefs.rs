//! Behaviour switches the audio thread reads every block.
//!
//! The Audio settings panel has offered these two since it was written and
//! said so in its own comment: "UI-only at the time of writing". They were
//! stored in the browser and nothing in the engine read them, so both
//! behaved as whatever the engine happened to do.
//!
//! Shared as atomics rather than passed at graph-rebuild time, so flipping a
//! switch takes effect on the next block instead of the next project change,
//! and the audio thread never waits on the UI to read one.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub struct AudioPrefs {
    /// Clear instrument voices and reset every node when the transport stops
    /// or the playhead jumps, so nothing from before the jump is still
    /// sounding.
    reset_on_transport: Arc<AtomicBool>,
    /// Play a note the playhead landed in the middle of, from the middle,
    /// rather than waiting for the next note to start.
    play_truncated_notes: Arc<AtomicBool>,
    /// Milliseconds added to the measured recording latency. Drivers do not
    /// always report all of it (converters, USB buffering), so a take can
    /// still sit a little late or early; this is the manual correction.
    record_offset_ms: Arc<AtomicI32>,
}

impl AudioPrefs {
    pub fn new() -> Self {
        Self {
            // Matches the panel's own defaults, so the engine and the UI
            // agree before the UI has said anything.
            reset_on_transport: Arc::new(AtomicBool::new(true)),
            play_truncated_notes: Arc::new(AtomicBool::new(false)),
            record_offset_ms: Arc::new(AtomicI32::new(0)),
        }
    }

    pub fn set_reset_on_transport(&self, on: bool) {
        self.reset_on_transport.store(on, Ordering::Relaxed);
    }

    pub fn reset_on_transport(&self) -> bool {
        self.reset_on_transport.load(Ordering::Relaxed)
    }

    pub fn set_play_truncated_notes(&self, on: bool) {
        self.play_truncated_notes.store(on, Ordering::Relaxed);
    }

    pub fn play_truncated_notes(&self) -> bool {
        self.play_truncated_notes.load(Ordering::Relaxed)
    }

    /// Clamped to half a second either way: anything larger is a mistake,
    /// not a converter.
    pub fn set_record_offset_ms(&self, ms: i32) {
        self.record_offset_ms
            .store(ms.clamp(-500, 500), Ordering::Relaxed);
    }

    pub fn record_offset_ms(&self) -> i32 {
        self.record_offset_ms.load(Ordering::Relaxed)
    }
}

impl Default for AudioPrefs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_match_the_settings_panel() {
        let prefs = AudioPrefs::new();
        assert!(prefs.reset_on_transport());
        assert!(!prefs.play_truncated_notes());
    }

    #[test]
    fn a_clone_shares_the_same_switches() {
        let prefs = AudioPrefs::new();
        let audio_side = prefs.clone();
        prefs.set_play_truncated_notes(true);
        assert!(
            audio_side.play_truncated_notes(),
            "the audio thread must see the change without a rebuild"
        );
    }
}
