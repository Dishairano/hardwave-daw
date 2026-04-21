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
use std::time::Instant;

use crate::MidiEvent;

/// Maximum number of events buffered before the oldest start dropping.
/// 4096 holds ~40s of CC spam at 100 Hz from a single controller, well above
/// anything the audio thread could miss in a normal drain cadence.
const QUEUE_CAPACITY: usize = 4096;

#[derive(Default)]
struct SharedState {
    events: VecDeque<MidiEvent>,
    last_event_at: Option<Instant>,
    dropped: u64,
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
                    if let Some(event) = parse_midi_bytes(0, bytes) {
                        let mut state = shared.lock();
                        if state.events.len() >= QUEUE_CAPACITY {
                            state.events.pop_front();
                            state.dropped = state.dropped.saturating_add(1);
                        }
                        state.events.push_back(event);
                        state.last_event_at = Some(Instant::now());
                    }
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
