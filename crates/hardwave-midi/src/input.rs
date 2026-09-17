//! Live MIDI input via midir — device enumeration, port open/close, and a
//! shared queue of parsed `MidiEvent`s for the engine/UI to drain.
//!
//! The midir callback runs on a background thread owned by the OS MIDI
//! subsystem. It pushes parsed events into a VecDeque behind a Mutex; the
//! engine drains that queue each audio block, and the Tauri commands read a
//! timestamp-only activity summary for the toolbar LED.

use midir::{MidiInput, MidiInputConnection};
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::velocity::VelocityCurve;
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

/// Result of a single reconcile pass — useful for logging/telemetry.
#[derive(Debug, Default, Clone)]
pub struct ReconcileReport {
    pub reopened: Vec<String>,
    pub disconnected: Vec<String>,
    pub failed: Vec<(String, String)>,
}

pub struct MidiInputManager {
    active: Vec<(String, MidiInputConnection<PortContext>)>,
    shared: Arc<Mutex<SharedState>>,
    /// Ports the user has asked to keep open. Survives disconnection so the
    /// reconciler can reopen them when the device reappears.
    desired: Vec<String>,
    /// Velocity curve per port, shared with the midir callback through an
    /// atomic so changing it applies to a port that is already open. Kept for
    /// ports that are not open yet as well, so the setting survives a
    /// disconnection and a reconnect.
    velocity_curves: HashMap<String, Arc<AtomicU8>>,
    /// The master "enable MIDI remote control" switch. The setup wizard has
    /// offered it since it was written, saying that with it off no MIDI input
    /// reaches the audio thread, and nothing read it. Ports stay open when it
    /// is off and their messages are dropped as they arrive, so turning it
    /// back on needs no reconnection.
    enabled: Arc<AtomicBool>,
}

/// What the midir callback for one port needs: the queue every port shares,
/// and that port's own velocity curve.
struct PortContext {
    shared: Arc<Mutex<SharedState>>,
    velocity_curve: Arc<AtomicU8>,
    enabled: Arc<AtomicBool>,
}

impl MidiInputManager {
    pub fn new() -> Self {
        Self {
            active: Vec::new(),
            shared: Arc::new(Mutex::new(SharedState::default())),
            desired: Vec::new(),
            velocity_curves: HashMap::new(),
            enabled: Arc::new(AtomicBool::new(true)),
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
    /// Adds the port to the desired set so the reconciler will reopen it
    /// automatically if the device is later unplugged and replugged.
    pub fn open(&mut self, port_name: &str) -> Result<(), String> {
        if !self.desired.iter().any(|n| n == port_name) {
            self.desired.push(port_name.to_string());
        }
        self.connect_one(port_name)
    }

    /// Establish the midir connection without touching the desired set — used
    /// both by `open` (after it updates desired) and by `reconcile` when a
    /// port reappears.
    fn connect_one(&mut self, port_name: &str) -> Result<(), String> {
        if self.is_open(port_name) {
            return Ok(());
        }
        let input = MidiInput::new("hardwave-midi").map_err(|e| format!("MidiInput::new: {e}"))?;
        let ports = input.ports();
        let port = ports
            .iter()
            .find(|p| input.port_name(p).ok().as_deref() == Some(port_name))
            .ok_or_else(|| format!("MIDI port not found: {port_name}"))?;

        let context = PortContext {
            shared: Arc::clone(&self.shared),
            velocity_curve: Arc::clone(self.velocity_curve_slot(port_name)),
            enabled: Arc::clone(&self.enabled),
        };
        let conn = input
            .connect(
                port,
                "hardwave-midi-in",
                move |_stamp, bytes, context: &mut PortContext| {
                    handle_input_bytes(bytes, context);
                },
                context,
            )
            .map_err(|e| format!("connect: {e}"))?;

        log::info!("Opened MIDI input port: {port_name}");
        self.active.push((port_name.to_string(), conn));
        Ok(())
    }

    /// Turn MIDI input on or off as a whole.
    ///
    /// With it off nothing reaches the queue the audio thread drains, so no
    /// note, controller move or incoming clock has any effect.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// The shared slot holding one port's velocity curve, created on demand.
    fn velocity_curve_slot(&mut self, port_name: &str) -> &Arc<AtomicU8> {
        self.velocity_curves
            .entry(port_name.to_string())
            .or_insert_with(|| Arc::new(AtomicU8::new(VelocityCurve::Linear.to_index())))
    }

    /// Set the velocity curve for one input port.
    ///
    /// Takes effect immediately on an open port, and is remembered for a port
    /// that is not open yet, so the wizard can apply a saved setting before
    /// the controller is plugged in.
    pub fn set_velocity_curve(&mut self, port_name: &str, curve: VelocityCurve) {
        self.velocity_curve_slot(port_name)
            .store(curve.to_index(), Ordering::Relaxed);
    }

    /// The curve in force for one port.
    pub fn velocity_curve(&self, port_name: &str) -> VelocityCurve {
        self.velocity_curves
            .get(port_name)
            .map(|slot| VelocityCurve::from_index(slot.load(Ordering::Relaxed)))
            .unwrap_or_default()
    }

    /// Close a port by display name. Removes it from the desired set so the
    /// reconciler doesn't reopen it.
    pub fn close(&mut self, port_name: &str) {
        self.active.retain(|(n, _)| n != port_name);
        self.desired.retain(|n| n != port_name);
    }

    /// Close every open port and clear desired.
    pub fn close_all(&mut self) {
        self.active.clear();
        self.desired.clear();
    }

    /// Ports the user has asked to keep open, regardless of whether the
    /// underlying device is currently plugged in.
    pub fn desired_port_names(&self) -> Vec<String> {
        self.desired.clone()
    }

    /// Single reconcile pass — reopens any desired port that is now available
    /// but not active, and drops active connections whose port has vanished
    /// from the system list. Called periodically by the hot-plug scanner.
    pub fn reconcile(&mut self) -> ReconcileReport {
        let mut report = ReconcileReport::default();
        let available = self.list_ports();

        // Drop active connections for ports that no longer exist.
        let active_names: Vec<String> = self.active.iter().map(|(n, _)| n.clone()).collect();
        for name in active_names {
            if !available.iter().any(|a| a == &name) {
                self.active.retain(|(n, _)| n != &name);
                log::info!("MIDI port vanished, dropped connection: {name}");
                report.disconnected.push(name);
            }
        }

        // Reopen desired ports that are available but not active.
        let desired_snapshot: Vec<String> = self.desired.clone();
        for name in desired_snapshot {
            if self.is_open(&name) {
                continue;
            }
            if !available.iter().any(|a| a == &name) {
                continue;
            }
            match self.connect_one(&name) {
                Ok(()) => {
                    log::info!("MIDI port reconnected: {name}");
                    report.reopened.push(name);
                }
                Err(e) => {
                    log::warn!("MIDI reconnect failed for {name}: {e}");
                    report.failed.push((name, e));
                }
            }
        }

        report
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

    /// Non-blocking drain into a caller-owned buffer. Returns true when the
    /// lock was acquired (events may still be empty). Returns false on
    /// contention — the audio thread calls this once per block and skipping
    /// a single block of MIDI input adds at most ~5ms of latency, which is
    /// inaudible compared to scheduler jitter. The midir callback path
    /// continues to push into `shared.events` via `handle_input_bytes`,
    /// so contended events are picked up on the next successful drain.
    pub fn try_drain_events_into(&self, out: &mut Vec<MidiEvent>) -> bool {
        let Some(mut state) = self.shared.try_lock() else {
            return false;
        };
        out.extend(state.events.drain(..));
        true
    }

    /// Inject a synthetic event into the same queue the midir callbacks
    /// write to. Used by the inject-MIDI Tauri command for the computer
    /// keyboard and on-screen virtual keyboard paths so they hit the
    /// exact same drain pipeline as a physical controller.
    pub fn inject(&self, ev: MidiEvent) {
        let mut state = self.shared.lock();
        if state.events.len() >= QUEUE_CAPACITY {
            state.events.pop_front();
            state.dropped = state.dropped.saturating_add(1);
        }
        state.events.push_back(ev);
        state.last_event_at = Some(Instant::now());
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
fn handle_input_bytes(bytes: &[u8], context: &PortContext) {
    if !context.enabled.load(Ordering::Relaxed) {
        return;
    }
    let shared = &context.shared;
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
                // The curve belongs to the port the bytes arrived on, so two
                // controllers with different feels can each be corrected.
                let event = apply_velocity_curve(
                    event,
                    VelocityCurve::from_index(context.velocity_curve.load(Ordering::Relaxed)),
                );
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

/// Shape a note's velocity with the port's curve.
///
/// Note-on only. A note-off velocity is a release velocity, which almost no
/// controller sends meaningfully and no synth here reads, and curving it would
/// change when notes are released rather than how hard they sound.
fn apply_velocity_curve(event: MidiEvent, curve: VelocityCurve) -> MidiEvent {
    if curve == VelocityCurve::Linear {
        return event;
    }
    match event {
        MidiEvent::NoteOn {
            timing,
            channel,
            note,
            velocity,
        } => MidiEvent::NoteOn {
            timing,
            channel,
            note,
            velocity: curve.apply(velocity),
        },
        other => other,
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
    fn a_velocity_curve_shapes_note_ons_only() {
        let note_on = parse_midi_bytes(0, &[0x90, 60, 32]).unwrap();
        let shaped = apply_velocity_curve(note_on, VelocityCurve::Soft);
        match shaped {
            MidiEvent::NoteOn { velocity, .. } => {
                // 32/127 is about 0.25, and the soft curve lifts it.
                assert!(velocity > 0.4, "soft curve did not lift it: {velocity}");
            }
            other => panic!("expected a note on, got {other:?}"),
        }

        // A release velocity is left alone: curving it would change when
        // notes end rather than how hard they sound.
        let note_off = parse_midi_bytes(0, &[0x80, 60, 32]).unwrap();
        let shaped_off = apply_velocity_curve(note_off, VelocityCurve::Soft);
        match shaped_off {
            MidiEvent::NoteOff { velocity, .. } => {
                assert!((velocity - 32.0 / 127.0).abs() < 1e-6, "{velocity}");
            }
            other => panic!("expected a note off, got {other:?}"),
        }
    }

    #[test]
    fn the_linear_curve_passes_the_controller_through_untouched() {
        let note_on = parse_midi_bytes(0, &[0x90, 60, 100]).unwrap();
        let shaped = apply_velocity_curve(note_on, VelocityCurve::Linear);
        match (note_on, shaped) {
            (MidiEvent::NoteOn { velocity: a, .. }, MidiEvent::NoteOn { velocity: b, .. }) => {
                assert_eq!(a, b)
            }
            _ => panic!("expected note ons"),
        }
    }

    #[test]
    fn the_master_switch_drops_everything_while_it_is_off() {
        let shared = Arc::new(Mutex::new(SharedState::default()));
        let enabled = Arc::new(AtomicBool::new(false));
        let context = PortContext {
            shared: Arc::clone(&shared),
            velocity_curve: Arc::new(AtomicU8::new(VelocityCurve::Linear.to_index())),
            enabled: Arc::clone(&enabled),
        };

        handle_input_bytes(&[0x90, 60, 100], &context);
        handle_input_bytes(&[0xF8], &context);
        {
            let state = shared.lock();
            assert!(state.events.is_empty(), "a note got through while off");
            assert!(state.last_tick_at.is_none(), "a clock tick got through");
        }

        // Back on, and the same note arrives, with no reconnection.
        enabled.store(true, Ordering::Relaxed);
        handle_input_bytes(&[0x90, 60, 100], &context);
        assert_eq!(shared.lock().events.len(), 1);
    }

    #[test]
    fn midi_input_is_on_until_it_is_switched_off() {
        let manager = MidiInputManager::new();
        assert!(manager.is_enabled());
        manager.set_enabled(false);
        assert!(!manager.is_enabled());
    }

    #[test]
    fn a_ports_curve_is_remembered_before_it_is_opened() {
        let mut manager = MidiInputManager::new();
        assert_eq!(manager.velocity_curve("Launchpad"), VelocityCurve::Linear);
        manager.set_velocity_curve("Launchpad", VelocityCurve::Hard);
        assert_eq!(manager.velocity_curve("Launchpad"), VelocityCurve::Hard);
        // Another controller keeps its own feel.
        assert_eq!(manager.velocity_curve("MPK Mini"), VelocityCurve::Linear);
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
