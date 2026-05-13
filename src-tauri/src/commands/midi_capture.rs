//! Rolling MIDI capture commands. Exposes the engine's always-on
//! capture ring (`MidiCaptureRing`) to the UI so the user can dump
//! the last N seconds of input into a new pattern without ever having
//! armed a track.
//!
//! The ring lives on the engine, the audio thread pushes per-block,
//! and these commands read snapshots on the UI thread under a
//! parking_lot Mutex try_lock.

use crate::AppState;
use serde::Serialize;
use tauri::State;

/// Serialised entry from the rolling ring. `sample_pos` is the absolute
/// transport sample at which the event was observed by the audio
/// thread. The UI converts to ticks using the project's tempo map.
#[derive(Serialize)]
pub struct CapturedMidiEntry {
    pub sample_pos: u64,
    pub kind: &'static str,
    pub channel: u8,
    pub note: Option<u8>,
    pub velocity: Option<f32>,
    pub cc: Option<u8>,
    pub value: Option<f32>,
}

impl CapturedMidiEntry {
    fn from(sample_pos: u64, ev: hardwave_midi::MidiEvent) -> Self {
        use hardwave_midi::MidiEvent;
        match ev {
            MidiEvent::NoteOn {
                channel,
                note,
                velocity,
                ..
            } => Self {
                sample_pos,
                kind: "note_on",
                channel,
                note: Some(note),
                velocity: Some(velocity),
                cc: None,
                value: None,
            },
            MidiEvent::NoteOff {
                channel,
                note,
                velocity,
                ..
            } => Self {
                sample_pos,
                kind: "note_off",
                channel,
                note: Some(note),
                velocity: Some(velocity),
                cc: None,
                value: None,
            },
            MidiEvent::ControlChange {
                channel, cc, value, ..
            } => Self {
                sample_pos,
                kind: "control_change",
                channel,
                note: None,
                velocity: None,
                cc: Some(cc),
                value: Some(value),
            },
            MidiEvent::PitchBend { channel, value, .. } => Self {
                sample_pos,
                kind: "pitch_bend",
                channel,
                note: None,
                velocity: None,
                cc: None,
                value: Some(value),
            },
            MidiEvent::Aftertouch {
                channel,
                note,
                pressure,
                ..
            } => Self {
                sample_pos,
                kind: "aftertouch",
                channel,
                note: Some(note),
                velocity: Some(pressure),
                cc: None,
                value: None,
            },
            MidiEvent::ChannelPressure {
                channel, pressure, ..
            } => Self {
                sample_pos,
                kind: "channel_pressure",
                channel,
                note: None,
                velocity: Some(pressure),
                cc: None,
                value: None,
            },
        }
    }
}

/// Return every event currently in the rolling capture ring, oldest
/// first. The UI filters by `sample_pos >= now - window_samples` to
/// implement "dump last 30 seconds" / "dump everything since I sat
/// down". Returns an empty Vec if the ring is busy (audio thread
/// pushing) — the UI can retry immediately.
#[tauri::command]
pub fn dump_midi_capture(state: State<AppState>) -> Vec<CapturedMidiEntry> {
    // Same Arc-clone-then-drop-engine pattern as clear_midi_capture to
    // keep the MutexGuard's borrow disjoint from the engine binding.
    let ring_arc = {
        let engine = state.engine.lock();
        std::sync::Arc::clone(&engine.midi_capture_ring)
    };
    let Some(ring) = ring_arc.try_lock() else {
        return Vec::new();
    };
    ring.entries_in_order()
        .into_iter()
        .map(|(pos, ev)| CapturedMidiEntry::from(pos, ev))
        .collect()
}

/// Wipe the capture ring. Used by the UI on project switch so the
/// next "dump last N seconds" doesn't smuggle events from the
/// previously-loaded session into the new one.
#[tauri::command]
pub fn clear_midi_capture(state: State<AppState>) {
    let ring_arc = {
        let engine = state.engine.lock();
        std::sync::Arc::clone(&engine.midi_capture_ring)
    };
    // Bind the Option<MutexGuard> to a NAMED local. With a let-binding
    // it's a proper variable rather than a tail-expression temporary,
    // so drop order is declaration-reverse: opt drops before ring_arc.
    let opt = ring_arc.try_lock();
    if let Some(mut ring) = opt {
        ring.clear();
    }
}
