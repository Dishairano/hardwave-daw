//! Live MIDI output via midir — device enumeration, port open/close, and raw
//! message sending. Each open port owns its own `MidiOutputConnection` wrapped
//! in a mutex so callers (engine transport thread, clock dispatcher, preview)
//! can share a single manager without coordinating directly.

use midir::{MidiOutput, MidiOutputConnection};
use parking_lot::Mutex;
use std::sync::Arc;

pub struct MidiOutputManager {
    active: Vec<(String, Arc<Mutex<MidiOutputConnection>>)>,
}

impl MidiOutputManager {
    pub fn new() -> Self {
        Self { active: Vec::new() }
    }

    /// List available MIDI output port names. Returns an empty vec if the
    /// system has no MIDI subsystem.
    pub fn list_ports(&self) -> Vec<String> {
        let output = match MidiOutput::new("hardwave-midi-out-scan") {
            Ok(o) => o,
            Err(e) => {
                log::warn!("MidiOutput::new failed during scan: {e}");
                return Vec::new();
            }
        };
        output
            .ports()
            .iter()
            .filter_map(|p| output.port_name(p).ok())
            .collect()
    }

    /// Open an output port by display name. No-op if already open.
    pub fn open(&mut self, port_name: &str) -> Result<(), String> {
        if self.is_open(port_name) {
            return Ok(());
        }
        let output =
            MidiOutput::new("hardwave-midi-out").map_err(|e| format!("MidiOutput::new: {e}"))?;
        let ports = output.ports();
        let port = ports
            .iter()
            .find(|p| output.port_name(p).ok().as_deref() == Some(port_name))
            .ok_or_else(|| format!("MIDI output port not found: {port_name}"))?;

        let conn = output
            .connect(port, "hardwave-midi-out")
            .map_err(|e| format!("connect: {e}"))?;
        log::info!("Opened MIDI output port: {port_name}");
        self.active
            .push((port_name.to_string(), Arc::new(Mutex::new(conn))));
        Ok(())
    }

    pub fn close(&mut self, port_name: &str) {
        self.active.retain(|(n, _)| n != port_name);
    }

    pub fn close_all(&mut self) {
        self.active.clear();
    }

    pub fn is_open(&self, port_name: &str) -> bool {
        self.active.iter().any(|(n, _)| n == port_name)
    }

    pub fn open_port_names(&self) -> Vec<String> {
        self.active.iter().map(|(n, _)| n.clone()).collect()
    }

    /// Send raw MIDI bytes to a specific open port. Returns `Err` if the
    /// port is not currently open.
    pub fn send(&self, port_name: &str, bytes: &[u8]) -> Result<(), String> {
        let conn = self
            .active
            .iter()
            .find(|(n, _)| n == port_name)
            .map(|(_, c)| Arc::clone(c))
            .ok_or_else(|| format!("MIDI output port not open: {port_name}"))?;
        let mut c = conn.lock();
        c.send(bytes).map_err(|e| format!("send: {e}"))
    }

    /// Send the same bytes to every open output port. Silently skips ports
    /// whose send fails — callers just want best-effort broadcast for clock.
    pub fn broadcast(&self, bytes: &[u8]) {
        for (name, conn) in self.active.iter() {
            let mut c = conn.lock();
            if let Err(e) = c.send(bytes) {
                log::debug!("MIDI broadcast to {name} failed: {e}");
            }
        }
    }
}

impl Default for MidiOutputManager {
    fn default() -> Self {
        Self::new()
    }
}

// Standard MIDI system realtime message bytes.
pub const MIDI_CLOCK_TICK: u8 = 0xF8;
pub const MIDI_START: u8 = 0xFA;
pub const MIDI_CONTINUE: u8 = 0xFB;
pub const MIDI_STOP: u8 = 0xFC;
