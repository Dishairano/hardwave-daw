//! External MIDI clock sync — dispatcher thread that observes clock-tick
//! statistics captured by the MIDI input manager and, when the user has
//! enabled sync, slaves the transport's BPM and play/stop state to the
//! external master.

use hardwave_engine::DawEngine;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub struct MidiClockSyncState {
    pub enabled: Arc<AtomicBool>,
    /// Most recent BPM observed on the external clock, or `None` if no
    /// clock master has been seen. Updated whether or not sync is enabled
    /// so the UI can show what *would* be slaved.
    pub last_bpm: Arc<Mutex<Option<f64>>>,
    /// True once any 0xF8 tick has been seen since the sync state was
    /// created. Lets the UI distinguish "no master present" from "master
    /// present, sync disabled".
    pub ticks_seen: Arc<AtomicBool>,
}

impl MidiClockSyncState {
    pub fn new() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            last_bpm: Arc::new(Mutex::new(None)),
            ticks_seen: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Default for MidiClockSyncState {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn the sync dispatcher. It drains the input manager's clock-sync
/// snapshot every ~10 ms and, when enabled, writes the estimated BPM to
/// `transport.bpm` and reacts to Start / Continue / Stop messages.
pub fn spawn_dispatcher(engine: Arc<Mutex<DawEngine>>, state: Arc<MidiClockSyncState>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(10));
        let snapshot = {
            let eng = engine.lock();
            let mgr = eng.midi_input.lock();
            mgr.take_clock_sync_snapshot()
        };

        if let Some(bpm) = snapshot.bpm_estimate {
            *state.last_bpm.lock() = Some(bpm);
        }
        if snapshot.ticks_received {
            state.ticks_seen.store(true, Ordering::Relaxed);
        }

        if !state.enabled.load(Ordering::Relaxed) {
            continue;
        }

        if let Some(bpm) = snapshot.bpm_estimate {
            let eng = engine.lock();
            eng.transport.bpm.store(bpm, Ordering::Relaxed);
        }
        if snapshot.pending_start {
            let eng = engine.lock();
            eng.transport.position_samples.store(0, Ordering::Relaxed);
            eng.transport.playing.store(true, Ordering::Relaxed);
        }
        if snapshot.pending_continue {
            let eng = engine.lock();
            eng.transport.playing.store(true, Ordering::Relaxed);
        }
        if snapshot.pending_stop {
            let eng = engine.lock();
            eng.transport.playing.store(false, Ordering::Relaxed);
        }
    });
}
