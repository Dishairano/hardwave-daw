use crate::AppState;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use tauri::State;

#[derive(Serialize)]
pub struct MidiActivitySnapshot {
    pub open_ports: Vec<String>,
    pub ms_since_last_event: Option<u64>,
}

#[derive(Serialize)]
pub struct MidiClockSyncStatus {
    pub enabled: bool,
    pub ticks_seen: bool,
    pub last_bpm: Option<f64>,
}

#[tauri::command]
pub fn list_midi_inputs(state: State<AppState>) -> Vec<String> {
    let engine = state.engine.lock();
    let manager = engine.midi_input.lock();
    manager.list_ports()
}

#[tauri::command]
pub fn open_midi_input(state: State<AppState>, port_name: String) -> Result<(), String> {
    let engine = state.engine.lock();
    let mut manager = engine.midi_input.lock();
    manager.open(&port_name)
}

#[tauri::command]
pub fn close_midi_input(state: State<AppState>, port_name: String) {
    let engine = state.engine.lock();
    let mut manager = engine.midi_input.lock();
    manager.close(&port_name);
}

#[tauri::command]
pub fn close_all_midi_inputs(state: State<AppState>) {
    let engine = state.engine.lock();
    let mut manager = engine.midi_input.lock();
    manager.close_all();
}

#[tauri::command]
pub fn get_midi_activity(state: State<AppState>) -> MidiActivitySnapshot {
    let engine = state.engine.lock();
    let manager = engine.midi_input.lock();
    MidiActivitySnapshot {
        open_ports: manager.open_port_names(),
        ms_since_last_event: manager.ms_since_last_event(),
    }
}

#[tauri::command]
pub fn get_midi_desired_ports(state: State<AppState>) -> Vec<String> {
    let engine = state.engine.lock();
    let manager = engine.midi_input.lock();
    manager.desired_port_names()
}

#[tauri::command]
pub fn set_midi_clock_sync_enabled(state: State<AppState>, enabled: bool) {
    state.midi_sync.enabled.store(enabled, Ordering::Relaxed);
}

#[tauri::command]
pub fn get_midi_clock_sync_status(state: State<AppState>) -> MidiClockSyncStatus {
    MidiClockSyncStatus {
        enabled: state.midi_sync.enabled.load(Ordering::Relaxed),
        ticks_seen: state.midi_sync.ticks_seen.load(Ordering::Relaxed),
        last_bpm: *state.midi_sync.last_bpm.lock(),
    }
}

/// Wire-format DTO for `inject_midi_event`. Mirrors the variant set of
/// `hardwave_midi::MidiEvent` 1:1 so the frontend can build a typed
/// payload without needing access to the Rust enum. Timing is in
/// per-block samples; on-screen/computer-keyboard injection always
/// passes 0 because the event reaches the audio thread as soon as the
/// drain loop runs.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MidiEventDto {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: f32,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    ControlChange {
        channel: u8,
        cc: u8,
        value: f32,
    },
    PitchBend {
        channel: u8,
        value: f32,
    },
}

impl MidiEventDto {
    fn into_event(self) -> hardwave_midi::MidiEvent {
        use hardwave_midi::MidiEvent;
        match self {
            MidiEventDto::NoteOn {
                channel,
                note,
                velocity,
            } => MidiEvent::NoteOn {
                timing: 0,
                channel,
                note,
                velocity: velocity.clamp(0.0, 1.0),
            },
            MidiEventDto::NoteOff { channel, note } => MidiEvent::NoteOff {
                timing: 0,
                channel,
                note,
                velocity: 0.0,
            },
            MidiEventDto::ControlChange { channel, cc, value } => MidiEvent::ControlChange {
                timing: 0,
                channel,
                cc,
                value: value.clamp(0.0, 1.0),
            },
            MidiEventDto::PitchBend { channel, value } => MidiEvent::PitchBend {
                timing: 0,
                channel,
                value: value.clamp(-1.0, 1.0),
            },
        }
    }
}

/// Inject a synthetic MIDI event into the engine's input pipeline.
/// Feeds the same shared queue as midir hardware callbacks, so the
/// on-screen keyboard, the computer-keyboard hook, and the MIDI-learn
/// dispatcher all observe events from these calls. Fire-and-forget —
/// errors are unrecoverable from the UI side (engine not started, lock
/// contention) so we silently no-op.
#[tauri::command]
pub fn inject_midi_event(state: State<AppState>, event: MidiEventDto) {
    let engine = state.engine.lock();
    let manager = engine.midi_input.lock();
    manager.inject(event.into_event());
}
