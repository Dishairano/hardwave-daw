//! Mackie Control and HUI: a desk with real faders driving the mixer.
//!
//! A control surface speaks ordinary MIDI, but not the MIDI Learn kind:
//! the faders are pitch bend, one channel each, and the buttons are
//! notes at fixed numbers. Learn could map one knob to one parameter;
//! it could never give you eight faders, their mutes and their solos
//! at once, and it could not move a motorised fader back.
//!
//! What is here is the common ground between the two protocols, which
//! is what every desk implements: eight faders, mute, solo and arm per
//! strip, the transport keys, and bank left and right. The scribble
//! strips and the LED rings are not: they need the sysex display
//! protocol, which differs between the two.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use hardwave_midi::MidiEvent;
use hardwave_project::Project;

/// Faders on a desk, and the master beside them.
const STRIPS: usize = 8;

/// Note numbers every Mackie-compatible desk sends.
const NOTE_REC_FIRST: u8 = 0; // 0..7   arm
const NOTE_SOLO_FIRST: u8 = 8; // 8..15  solo
const NOTE_MUTE_FIRST: u8 = 16; // 16..23 mute
const NOTE_BANK_LEFT: u8 = 46;
const NOTE_BANK_RIGHT: u8 = 47;
const NOTE_REWIND: u8 = 91;
const NOTE_FORWARD: u8 = 92;
const NOTE_STOP: u8 = 93;
const NOTE_PLAY: u8 = 94;
const NOTE_RECORD: u8 = 95;

/// What the desk is doing: whether it is listened to at all, and which
/// eight tracks its strips are on.
#[derive(Default)]
pub struct ControlSurface {
    pub enabled: AtomicBool,
    /// Index of the first track under the strips.
    pub bank: AtomicUsize,
}

impl ControlSurface {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
}

/// What one message from the desk asks for.
///
/// Worked out without touching the engine so it can be tested on its
/// own: the desk's protocol is the part that is easy to get wrong.
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceAction {
    Fader {
        strip: usize,
        value: f32,
    },
    MasterFader {
        value: f32,
    },
    Mute {
        strip: usize,
    },
    Solo {
        strip: usize,
    },
    Arm {
        strip: usize,
    },
    BankLeft,
    BankRight,
    Play,
    Stop,
    Record,
    Rewind,
    Forward,
    /// Set the tempo. No desk sends this, but a phone running TouchOSC
    /// does, and it is the same kind of message.
    Tempo {
        bpm: f64,
    },
    Pan {
        strip: usize,
        value: f32,
    },
    /// Jump the playhead, in beats from the start.
    Goto {
        beats: f64,
    },
}

/// Read one MIDI message as a desk message, or nothing when it is not
/// one. Note-offs are ignored: a desk sends a pair per button press and
/// acting on both would toggle twice.
pub fn interpret(event: &MidiEvent) -> Option<SurfaceAction> {
    match *event {
        MidiEvent::PitchBend { channel, value, .. } => {
            // Pitch bend arrives as -1..1; a fader is 0..1.
            let level = ((value + 1.0) * 0.5).clamp(0.0, 1.0);
            if (channel as usize) < STRIPS {
                Some(SurfaceAction::Fader {
                    strip: channel as usize,
                    value: level,
                })
            } else if channel as usize == STRIPS {
                Some(SurfaceAction::MasterFader { value: level })
            } else {
                None
            }
        }
        MidiEvent::NoteOn { note, velocity, .. } => {
            if velocity <= 0.0 {
                return None;
            }
            match note {
                NOTE_REC_FIRST..=7 => Some(SurfaceAction::Arm {
                    strip: (note - NOTE_REC_FIRST) as usize,
                }),
                NOTE_SOLO_FIRST..=15 => Some(SurfaceAction::Solo {
                    strip: (note - NOTE_SOLO_FIRST) as usize,
                }),
                NOTE_MUTE_FIRST..=23 => Some(SurfaceAction::Mute {
                    strip: (note - NOTE_MUTE_FIRST) as usize,
                }),
                NOTE_BANK_LEFT => Some(SurfaceAction::BankLeft),
                NOTE_BANK_RIGHT => Some(SurfaceAction::BankRight),
                NOTE_REWIND => Some(SurfaceAction::Rewind),
                NOTE_FORWARD => Some(SurfaceAction::Forward),
                NOTE_STOP => Some(SurfaceAction::Stop),
                NOTE_PLAY => Some(SurfaceAction::Play),
                NOTE_RECORD => Some(SurfaceAction::Record),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A fader position as the three bytes a desk expects back, so a
/// motorised fader moves when the mix changes rather than sitting where
/// the hand left it.
pub fn fader_feedback(strip: usize, value: f32) -> [u8; 3] {
    let level = value.clamp(0.0, 1.0);
    let raw = (level * 16383.0).round() as u16;
    [
        0xE0 | (strip as u8 & 0x0F),
        (raw & 0x7F) as u8,
        ((raw >> 7) & 0x7F) as u8,
    ]
}

/// A lit or dark button, as the desk expects it back.
pub fn button_feedback(note: u8, lit: bool) -> [u8; 3] {
    [0x90, note, if lit { 127 } else { 0 }]
}

/// The note that lights a strip's mute button.
pub fn mute_note(strip: usize) -> u8 {
    NOTE_MUTE_FIRST + (strip as u8 & 0x07)
}

/// Which tracks the eight strips are on, as ids.
///
/// Master is not one of them: it has the desk's own master fader, and
/// having it under a strip as well would mean two faders fighting.
pub fn bank_tracks(project: &Project, bank: usize) -> Vec<String> {
    project
        .tracks
        .iter()
        .filter(|t| t.kind.is_audio_bearing())
        .skip(bank * STRIPS)
        .take(STRIPS)
        .map(|t| t.id.clone())
        .collect()
}

/// How many banks of eight the song has, at least one.
pub fn bank_count(project: &Project) -> usize {
    let tracks = project
        .tracks
        .iter()
        .filter(|t| t.kind.is_audio_bearing())
        .count();
    tracks.div_ceil(STRIPS).max(1)
}

/// A fader position in decibels, the way the mixer stores it.
///
/// The curve matches the one MIDI Learn uses for a track fader, so the
/// desk and a learned knob move a fader the same distance.
pub fn fader_to_db(value: f32) -> f64 {
    // clamp keeps NaN as NaN; a broken value is the fader at the bottom.
    let value = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    -60.0 + value as f64 * 60.0
}

/// The reverse, for sending a fader back to the desk.
pub fn db_to_fader(db: f64) -> f32 {
    (((db + 60.0) / 60.0) as f32).clamp(0.0, 1.0)
}

/// Shared handle for the app and the dispatcher.
pub type SharedSurface = Arc<ControlSurface>;

#[cfg(test)]
mod tests {
    use super::*;

    fn bend(channel: u8, value: f32) -> MidiEvent {
        MidiEvent::PitchBend {
            timing: 0,
            channel,
            value,
        }
    }

    fn note_on(note: u8) -> MidiEvent {
        MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note,
            velocity: 1.0,
        }
    }

    #[test]
    fn a_fader_is_pitch_bend_on_its_own_channel() {
        assert_eq!(
            interpret(&bend(2, 0.0)),
            Some(SurfaceAction::Fader {
                strip: 2,
                value: 0.5
            })
        );
        assert_eq!(
            interpret(&bend(0, -1.0)),
            Some(SurfaceAction::Fader {
                strip: 0,
                value: 0.0
            })
        );
    }

    #[test]
    fn the_ninth_fader_is_the_master() {
        assert_eq!(
            interpret(&bend(8, 1.0)),
            Some(SurfaceAction::MasterFader { value: 1.0 })
        );
    }

    #[test]
    fn the_buttons_are_notes_at_the_numbers_every_desk_sends() {
        assert_eq!(
            interpret(&note_on(0)),
            Some(SurfaceAction::Arm { strip: 0 })
        );
        assert_eq!(
            interpret(&note_on(11)),
            Some(SurfaceAction::Solo { strip: 3 })
        );
        assert_eq!(
            interpret(&note_on(23)),
            Some(SurfaceAction::Mute { strip: 7 })
        );
        assert_eq!(interpret(&note_on(94)), Some(SurfaceAction::Play));
        assert_eq!(interpret(&note_on(47)), Some(SurfaceAction::BankRight));
    }

    #[test]
    fn a_note_off_does_nothing() {
        let off = MidiEvent::NoteOff {
            timing: 0,
            channel: 0,
            note: 16,
            velocity: 0.0,
        };
        assert_eq!(interpret(&off), None);
        // A desk that sends note-on with no velocity means the same.
        let zero = MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 16,
            velocity: 0.0,
        };
        assert_eq!(interpret(&zero), None);
    }

    #[test]
    fn a_fader_sent_back_is_the_same_position_it_came_in_at() {
        for value in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let bytes = fader_feedback(3, value);
            assert_eq!(bytes[0], 0xE3);
            let raw = (bytes[1] as u16) | ((bytes[2] as u16) << 7);
            let back = raw as f32 / 16383.0;
            assert!((back - value).abs() < 0.001, "{value} came back as {back}");
        }
    }

    #[test]
    fn a_fader_and_its_level_agree_in_both_directions() {
        for value in [0.0f32, 0.3, 1.0] {
            let db = fader_to_db(value);
            assert!((db_to_fader(db) - value).abs() < 1e-6);
        }
    }
}
