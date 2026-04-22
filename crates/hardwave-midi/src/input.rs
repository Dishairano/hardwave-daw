//! Live MIDI input via midir — device enumeration, port open/close, and a
//! shared queue of parsed `MidiEvent`s for the engine/UI to drain.
//!
//! The midir callback runs on a background thread owned by the OS MIDI
//! subsystem. It pushes parsed events into a VecDeque behind a Mutex; the
//! engine drains that queue each audio block, and the Tauri commands read a
//! timestamp-only activity summary for the toolbar LED.

use midir::{MidiInput, MidiInputConnection};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::MidiEvent;

/// Maximum number of events buffered before the oldest start dropping.
/// 4096 holds ~40s of CC spam at 100 Hz from a single controller, well above
/// anything the audio thread could miss in a normal drain cadence.
const QUEUE_CAPACITY: usize = 4096;

/// Number of recent clock-tick intervals averaged to produce the BPM
/// estimate. 48 ticks = two quarter notes at 24 PPQN, enough to smooth
/// jitter without reacting sluggishly to real tempo changes.
const TICK_HISTORY: usize = 48;

/// Snapshot of clock-sync observations drained from the shared state. The
/// `pending_*` flags are consumed by `take_clock_sync_snapshot` so each
/// start/stop/continue message drives exactly one transport action.
#[derive(Debug, Clone, Default)]
pub struct ClockSyncSnapshot {
    pub bpm_estimate: Option<f64>,
    pub ticks_received: bool,
    pub pending_start: bool,
    pub pending_continue: bool,
    pub pending_stop: bool,
}

#[derive(Default)]
struct SharedState {
    events: VecDeque<MidiEvent>,
    last_event_at: Option<Instant>,
    dropped: u64,
    // Clock sync — updated in the midir callback on system realtime bytes.
    last_tick_at: Option<Instant>,
    tick_intervals: VecDeque<Duration>,
    bpm_estimate: Option<f64>,
    pending_start: bool,
    pending_continue: bool,
    pending_stop: bool,
}

pub struct MidiInputManager {
    active: Vec<(String, MidiInputConnection<Arc<Mutex<SharedState>>>)>,
    shared: Arc<Mutex<SharedState>>,
}

impl MidiInputManager {
    pub fn new() -> Self {
        Self {
            active: Vec::new(),
            shared: Arc::new(Mutex::new(SharedState::default())),
        }
    }

    /// List available MIDI input port names. Returns an empty vec if the
    /// system has no MIDI subsystem (e.g. a Linux build without ALSA).
    pub fn list_ports(&self) -> Vec<String> {
        let input = match MidiInput::new("hardwave-midi-scan") {
            Ok(i) => i,
            Err(e) => {
                log::warn!("MidiInput::new failed during scan: {e}");
                return Vec::new();
            }
        };
        input
            .ports()
            .iter()
            .filter_map(|p| input.port_name(p).ok())
            .collect()
    }

    /// Open a port by its display name. No-op if the port is already open.
    pub fn open(&mut self, port_name: &str) -> Result<(), String> {
        if self.is_open(port_name) {
            return Ok(());
        }
        let input = MidiInput::new("hardwave-midi").map_err(|e| format!("MidiInput::new: {e}"))?;
        let ports = input.ports();
        let port = ports
            .iter()
            .find(|p| input.port_name(p).ok().as_deref() == Some(port_name))
            .ok_or_else(|| format!("MIDI port not found: {port_name}"))?;

        let shared = Arc::clone(&self.shared);
        let conn = input
            .connect(
                port,
                "hardwave-midi-in",
                move |_stamp, bytes, shared| {
                    handle_input_bytes(bytes, shared);
                },
                shared,
            )
            .map_err(|e| format!("connect: {e}"))?;

        log::info!("Opened MIDI input port: {port_name}");
        self.active.push((port_name.to_string(), conn));
        Ok(())
    }

    /// Close a port by display name.
    pub fn close(&mut self, port_name: &str) {
        self.active.retain(|(n, _)| n != port_name);
    }

    /// Close every open port.
    pub fn close_all(&mut self) {
        self.active.clear();
    }

    pub fn is_open(&self, port_name: &str) -> bool {
        self.active.iter().any(|(n, _)| n == port_name)
    }

    /// Names of currently-open ports.
    pub fn open_port_names(&self) -> Vec<String> {
        self.active.iter().map(|(n, _)| n.clone()).collect()
    }

    /// Drain and return every buffered event. Called by the engine once per
    /// audio block so events stay fresh even with many ports open.
    pub fn drain_events(&self) -> Vec<MidiEvent> {
        let mut state = self.shared.lock();
        state.events.drain(..).collect()
    }

    /// Return the current clock-sync observation and clear the pending
    /// transport flags so each Start/Continue/Stop message drives exactly
    /// one transport action in the caller.
    pub fn take_clock_sync_snapshot(&self) -> ClockSyncSnapshot {
        let mut state = self.shared.lock();
        let snap = ClockSyncSnapshot {
            bpm_estimate: state.bpm_estimate,
            ticks_received: state.last_tick_at.is_some(),
            pending_start: state.pending_start,
            pending_continue: state.pending_continue,
            pending_stop: state.pending_stop,
        };
        state.pending_start = false;
        state.pending_continue = false;
        state.pending_stop = false;
        snap
    }

    /// Milliseconds since the last event was seen, or None if no event has
    /// ever arrived since the manager was created.
    pub fn ms_since_last_event(&self) -> Option<u64> {
        let state = self.shared.lock();
        state
            .last_event_at
            .map(|t| t.elapsed().as_millis().min(u64::MAX as u128) as u64)
    }
}

impl Default for MidiInputManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Route one wire-format input message: clock and transport system-realtime
/// bytes update the shared sync state; every other message goes through the
/// normal `parse_midi_bytes` + event queue path.
fn handle_input_bytes(bytes: &[u8], shared: &Arc<Mutex<SharedState>>) {
    let now = Instant::now();
    match bytes.first().copied() {
        Some(0xF8) => {
            let mut state = shared.lock();
            if let Some(last) = state.last_tick_at {
                let delta = now.saturating_duration_since(last);
                if state.tick_intervals.len() >= TICK_HISTORY {
                    state.tick_intervals.pop_front();
                }
                state.tick_intervals.push_back(delta);
                if state.tick_intervals.len() >= 12 {
                    let sum: Duration = state.tick_intervals.iter().sum();
                    let avg_secs = sum.as_secs_f64() / state.tick_intervals.len() as f64;
                    if avg_secs > 0.0 {
                        let bpm = 60.0 / avg_secs / 24.0;
                        if bpm.is_finite() && (1.0..=999.0).contains(&bpm) {
                            state.bpm_estimate = Some(bpm);
                        }
                    }
                }
            }
            state.last_tick_at = Some(now);
            state.last_event_at = Some(now);
        }
        Some(0xFA) => {
            let mut state = shared.lock();
            state.pending_start = true;
            state.tick_intervals.clear();
            state.last_tick_at = None;
            state.last_event_at = Some(now);
        }
        Some(0xFB) => {
            let mut state = shared.lock();
            state.pending_continue = true;
            state.last_event_at = Some(now);
        }
        Some(0xFC) => {
            let mut state = shared.lock();
            state.pending_stop = true;
            state.last_event_at = Some(now);
        }
        _ => {
            if let Some(event) = parse_midi_bytes(0, bytes) {
                let mut state = shared.lock();
                if state.events.len() >= QUEUE_CAPACITY {
                    state.events.pop_front();
                    state.dropped = state.dropped.saturating_add(1);
                }
                state.events.push_back(event);
                state.last_event_at = Some(now);
            }
        }
    }
}

/// Parse one MIDI wire-format message. Returns None for status bytes we
/// don't model (realtime clock, SysEx, etc.).
pub fn parse_midi_bytes(timing: u32, bytes: &[u8]) -> Option<MidiEvent> {
    let status = *bytes.first()?;
    let channel = status & 0x0F;
    let msg_type = status & 0xF0;
    match msg_type {
        0x80 => {
            let note = *bytes.get(1)?;
            let velocity = *bytes.get(2)? as f32 / 127.0;
            Some(MidiEvent::NoteOff {
                timing,
                channel,
                note,
                velocity,
            })
        }
        0x90 => {
            let note = *bytes.get(1)?;
            let velocity = *bytes.get(2)? as f32 / 127.0;
            // Note On with velocity 0 is the conventional Note Off.
            if velocity == 0.0 {
                Some(MidiEvent::NoteOff {
                    timing,
                    channel,
                    note,
                    velocity: 0.0,
                })
            } else {
                Some(MidiEvent::NoteOn {
                    timing,
                    channel,
                    note,
                    velocity,
                })
            }
        }
        0xA0 => {
            let note = *bytes.get(1)?;
            let pressure = *bytes.get(2)? as f32 / 127.0;
            Some(MidiEvent::Aftertouch {
                timing,
                channel,
                note,
                pressure,
            })
        }
        0xB0 => {
            let cc = *bytes.get(1)?;
            let value = *bytes.get(2)? as f32 / 127.0;
            Some(MidiEvent::ControlChange {
                timing,
                channel,
                cc,
                value,
            })
        }
        0xD0 => {
            let pressure = *bytes.get(1)? as f32 / 127.0;
            Some(MidiEvent::ChannelPressure {
                timing,
                channel,
                pressure,
            })
        }
        0xE0 => {
            let lsb = *bytes.get(1)? as u32;
            let msb = *bytes.get(2)? as u32;
            let raw = (msb << 7) | lsb;
            let value = ((raw as f32) - 8192.0) / 8192.0;
            Some(MidiEvent::PitchBend {
                timing,
                channel,
                value,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_note_on() {
        let ev = parse_midi_bytes(0, &[0x90, 60, 100]).unwrap();
        match ev {
            MidiEvent::NoteOn {
                channel,
                note,
                velocity,
                ..
            } => {
                assert_eq!(channel, 0);
                assert_eq!(note, 60);
                assert!((velocity - 100.0 / 127.0).abs() < 1e-6);
            }
            _ => panic!("expected NoteOn"),
        }
    }

    #[test]
    fn note_on_velocity_zero_is_note_off() {
        let ev = parse_midi_bytes(0, &[0x90, 60, 0]).unwrap();
        assert!(matches!(ev, MidiEvent::NoteOff { .. }));
    }

    #[test]
    fn parses_pitch_bend_center() {
        let ev = parse_midi_bytes(0, &[0xE0, 0x00, 0x40]).unwrap();
        match ev {
            MidiEvent::PitchBend { value, .. } => assert!(value.abs() < 1e-3),
            _ => panic!("expected PitchBend"),
        }
    }

    #[test]
    fn parses_pitch_bend_full_down() {
        let ev = parse_midi_bytes(0, &[0xE0, 0x00, 0x00]).unwrap();
        match ev {
            MidiEvent::PitchBend { value, .. } => assert!((value + 1.0).abs() < 1e-3),
            _ => panic!("expected PitchBend"),
        }
    }

    #[test]
    fn ignores_unknown_status() {
        assert!(parse_midi_bytes(0, &[0xF8]).is_none());
    }
}
