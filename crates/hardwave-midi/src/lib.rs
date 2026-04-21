//! Hardwave MIDI — MIDI I/O, event types, and quantization.

pub mod input;
pub use input::MidiInputManager;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// MIDI events (modeled after nih-plug NoteEvent)
// ---------------------------------------------------------------------------

/// Sample-accurate MIDI event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MidiEvent {
    NoteOn {
        timing: u32,
        channel: u8,
        note: u8,
        velocity: f32,
    },
    NoteOff {
        timing: u32,
        channel: u8,
        note: u8,
        velocity: f32,
    },
    ControlChange {
        timing: u32,
        channel: u8,
        cc: u8,
        value: f32,
    },
    PitchBend {
        timing: u32,
        channel: u8,
        value: f32,
    },
    Aftertouch {
        timing: u32,
        channel: u8,
        note: u8,
        pressure: f32,
    },
    ChannelPressure {
        timing: u32,
        channel: u8,
        pressure: f32,
    },
}

impl MidiEvent {
    pub fn timing(&self) -> u32 {
        match self {
            Self::NoteOn { timing, .. } => *timing,
            Self::NoteOff { timing, .. } => *timing,
            Self::ControlChange { timing, .. } => *timing,
            Self::PitchBend { timing, .. } => *timing,
            Self::Aftertouch { timing, .. } => *timing,
            Self::ChannelPressure { timing, .. } => *timing,
        }
    }
}

// ---------------------------------------------------------------------------
// MIDI note (for piano roll / clip data)
// ---------------------------------------------------------------------------

/// A note in a MIDI clip. Uses ticks (960 PPQ).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidiNote {
    pub start_tick: u64,
    pub duration_ticks: u64,
    pub pitch: u8,
    pub velocity: f32,
    pub channel: u8,
    pub muted: bool,
}

/// A MIDI clip containing notes and CC data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidiClip {
    pub id: String,
    pub name: String,
    pub notes: Vec<MidiNote>,
    pub length_ticks: u64,
}

impl MidiClip {
    pub fn new(id: String, name: String, length_ticks: u64) -> Self {
        Self {
            id,
            name,
            notes: Vec::new(),
            length_ticks,
        }
    }
}

// ---------------------------------------------------------------------------
// Quantization
// ---------------------------------------------------------------------------

/// PPQ (pulses per quarter note).
pub const PPQ: u64 = 960;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum GridDivision {
    Bar,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    TripletQuarter,
    TripletEighth,
    TripletSixteenth,
}

impl GridDivision {
    /// Grid size in ticks (at 4/4 time signature).
    pub fn ticks(&self) -> u64 {
        match self {
            Self::Bar => PPQ * 4,
            Self::Half => PPQ * 2,
            Self::Quarter => PPQ,
            Self::Eighth => PPQ / 2,
            Self::Sixteenth => PPQ / 4,
            Self::ThirtySecond => PPQ / 8,
            Self::TripletQuarter => PPQ * 2 / 3,
            Self::TripletEighth => PPQ / 3,
            Self::TripletSixteenth => PPQ / 6,
        }
    }
}

pub struct QuantizeSettings {
    pub grid: GridDivision,
    pub strength: f64,
    pub swing: f64,
}

pub fn quantize_notes(notes: &mut [MidiNote], settings: &QuantizeSettings) {
    let grid = settings.grid.ticks();
    for note in notes.iter_mut() {
        if note.muted {
            continue;
        }
        let nearest = ((note.start_tick as f64 / grid as f64).round() * grid as f64) as u64;
        let offset = nearest as f64 - note.start_tick as f64;
        note.start_tick = (note.start_tick as f64 + offset * settings.strength) as u64;
    }
}
