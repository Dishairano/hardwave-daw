//! MIDI clock output — dispatcher thread that sends 24 PPQN clock ticks and
//! Start/Stop/Continue system realtime messages to every open MIDI output
//! port whenever clock send is enabled.
//!
//! The thread sleeps in short bursts (~500 µs) and computes the next tick
//! deadline from the live BPM at each iteration so tempo changes propagate
//! on the next tick without any coordination.

use hardwave_engine::DawEngine;
use hardwave_midi::{MidiOutputManager, MIDI_CLOCK_TICK, MIDI_START, MIDI_STOP};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct MidiClockState {
    pub enabled: Arc<AtomicBool>,
    pub output: Arc<Mutex<MidiOutputManager>>,
}

impl MidiClockState {
    pub fn new() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            output: Arc::new(Mutex::new(MidiOutputManager::new())),
        }
    }
}

impl Default for MidiClockState {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute the period between 24 PPQN clock ticks at a given BPM.
fn tick_interval(bpm: f64) -> Duration {
    let bpm = bpm.clamp(1.0, 999.0);
    Duration::from_secs_f64(60.0 / bpm / 24.0)
}

/// Spawn the MIDI clock dispatcher. Stays alive for the lifetime of the
/// process — the manager's ports decide whether anything actually hears the
/// ticks.
pub fn spawn_dispatcher(engine: Arc<Mutex<DawEngine>>, state: Arc<MidiClockState>) {
    std::thread::spawn(move || {
        let mut last_playing = false;
        let mut next_tick_at = Instant::now();
        loop {
            std::thread::sleep(Duration::from_micros(500));
            let enabled = state.enabled.load(Ordering::Relaxed);
            let (is_playing, bpm) = {
                let eng = engine.lock();
                let t = &eng.transport;
                (t.is_playing(), t.bpm.load(Ordering::Relaxed))
            };

            // Transport state change: emit Start when playback begins, Stop
            // when it ends. MIDI Continue is reserved for future resume-from-
            // non-zero handling and not currently emitted separately from
            // Start since the engine does not distinguish paused from stopped.
            if enabled && is_playing != last_playing {
                let out = state.output.lock();
                if is_playing {
                    out.broadcast(&[MIDI_START]);
                    next_tick_at = Instant::now();
                } else {
                    out.broadcast(&[MIDI_STOP]);
                }
            }
            last_playing = is_playing;

            if !enabled || !is_playing {
                continue;
            }

            let now = Instant::now();
            if now >= next_tick_at {
                {
                    let out = state.output.lock();
                    out.broadcast(&[MIDI_CLOCK_TICK]);
                }
                // Schedule the next tick from the previous target instant so
                // small jitter does not accumulate drift over time.
                next_tick_at += tick_interval(bpm);
                // If we fell more than a beat behind (e.g. BPM was changed
                // while the engine was paused), snap forward to avoid a burst.
                if now.saturating_duration_since(next_tick_at) > Duration::from_millis(100) {
                    next_tick_at = now + tick_interval(bpm);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_interval_at_120_bpm_is_half_period_of_quarter_note_over_24() {
        let i = tick_interval(120.0);
        let expected = 60.0 / 120.0 / 24.0;
        let actual = i.as_secs_f64();
        assert!((actual - expected).abs() < 1e-9);
    }

    #[test]
    fn tick_interval_clamps_bpm() {
        let low = tick_interval(0.0);
        let high = tick_interval(1e9);
        assert!(low.as_secs_f64() > 0.0);
        assert!(high.as_secs_f64() > 0.0);
    }
}
