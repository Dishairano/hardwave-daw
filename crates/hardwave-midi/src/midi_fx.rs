//! MIDI effects that sit before the instrument.
//!
//! An arpeggiator, a chord maker and a scale snapper already existed as
//! one-off tools that rewrote the notes in a clip. Rewriting is a
//! different thing from an effect: the part on the screen stops matching
//! what was played, and taking the arp off again means undoing rather
//! than switching it off.
//!
//! These run on the way to the instrument. The clip keeps the notes that
//! were written; the chain decides what the synth hears.

use serde::{Deserialize, Serialize};

use crate::MidiNote;

/// How an arpeggiator walks through the notes it is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArpMode {
    Up,
    Down,
    UpDown,
    /// The order the notes were written in, which for a chord is the
    /// order they sit in the clip.
    AsPlayed,
}

/// Which notes a scale keeps. Everything else is moved to the nearest
/// one that belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScaleKind {
    Major,
    NaturalMinor,
    HarmonicMinor,
    PhrygianDominant,
    Pentatonic,
}

impl ScaleKind {
    /// Semitone offsets from the root, inside one octave.
    pub fn degrees(self) -> &'static [u8] {
        match self {
            ScaleKind::Major => &[0, 2, 4, 5, 7, 9, 11],
            ScaleKind::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
            ScaleKind::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            // The hard dance staple: the scale most screeches are built on.
            ScaleKind::PhrygianDominant => &[0, 1, 4, 5, 7, 8, 10],
            ScaleKind::Pentatonic => &[0, 3, 5, 7, 10],
        }
    }
}

/// One effect in the chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MidiFx {
    /// Hold a chord, hear it one note at a time.
    Arpeggiator {
        /// Length of one arp step in ticks.
        step_ticks: u64,
        mode: ArpMode,
        /// How much of each step the note fills, 0.05 to 1.
        gate: f32,
        /// How many octaves the pattern climbs before repeating.
        octaves: u8,
    },
    /// Play one note, hear a chord.
    Chord {
        /// Semitones above the played note. An empty list changes nothing.
        intervals: Vec<i8>,
    },
    /// Move every note onto the nearest note of a scale.
    Scale {
        /// 0 = C, 1 = C#, and so on.
        root: u8,
        kind: ScaleKind,
    },
    /// Move everything by a fixed number of semitones.
    Transpose { semitones: i8 },
}

impl MidiFx {
    pub fn name(&self) -> &'static str {
        match self {
            MidiFx::Arpeggiator { .. } => "Arpeggiator",
            MidiFx::Chord { .. } => "Chord",
            MidiFx::Scale { .. } => "Scale",
            MidiFx::Transpose { .. } => "Transpose",
        }
    }
}

/// Nearest pitch that belongs to the scale.
///
/// A tie goes down, so a note exactly between two scale notes always
/// lands the same way instead of wandering with rounding.
pub fn snap_to_scale(pitch: u8, root: u8, kind: ScaleKind) -> u8 {
    let degrees = kind.degrees();
    let root = root % 12;
    let mut best = pitch;
    let mut best_distance = i32::MAX;
    // Look one octave either side so a note just under the root still
    // finds the degree above it.
    for octave in -1i32..=1 {
        for degree in degrees {
            let base = (pitch as i32 / 12) * 12 + root as i32 + *degree as i32 + octave * 12;
            if !(0..=127).contains(&base) {
                continue;
            }
            let distance = (base - pitch as i32).abs();
            if distance < best_distance {
                best_distance = distance;
                best = base as u8;
            }
        }
    }
    best
}

fn clamp_pitch(value: i32) -> Option<u8> {
    if (0..=127).contains(&value) {
        Some(value as u8)
    } else {
        None
    }
}

/// Notes that sound at the same time, treated as one chord by the
/// arpeggiator. Two notes count as together when they start within a
/// thirty-second note of each other.
const CHORD_WINDOW_TICKS: u64 = crate::PPQ / 8;

fn arpeggiate(
    notes: &[MidiNote],
    step_ticks: u64,
    mode: ArpMode,
    gate: f32,
    octaves: u8,
) -> Vec<MidiNote> {
    let step = step_ticks.max(1);
    let gate = gate.clamp(0.05, 1.0);
    let octaves = octaves.clamp(1, 4);

    let mut sorted: Vec<&MidiNote> = notes.iter().collect();
    sorted.sort_by_key(|n| (n.start_tick, n.pitch));

    let mut out: Vec<MidiNote> = Vec::new();
    let mut index = 0usize;
    while index < sorted.len() {
        // Gather the chord starting here.
        let start = sorted[index].start_tick;
        let mut group: Vec<&MidiNote> = Vec::new();
        while index < sorted.len() && sorted[index].start_tick <= start + CHORD_WINDOW_TICKS {
            group.push(sorted[index]);
            index += 1;
        }
        if group.iter().all(|n| n.muted) {
            continue;
        }
        // The chord lasts as long as its longest note.
        let end = group
            .iter()
            .map(|n| n.start_tick + n.duration_ticks)
            .max()
            .unwrap_or(start);
        if end <= start {
            continue;
        }

        let mut pitches: Vec<u8> = group.iter().filter(|n| !n.muted).map(|n| n.pitch).collect();
        pitches.sort_unstable();
        pitches.dedup();
        if pitches.is_empty() {
            continue;
        }
        let as_played: Vec<u8> = group.iter().filter(|n| !n.muted).map(|n| n.pitch).collect();

        // The walk, one octave at a time.
        let mut order: Vec<u8> = Vec::new();
        for octave in 0..octaves {
            let lift = octave as i32 * 12;
            let round: Vec<u8> = match mode {
                ArpMode::Up => pitches.clone(),
                ArpMode::Down => pitches.iter().rev().copied().collect(),
                ArpMode::UpDown => {
                    let mut v = pitches.clone();
                    // The top and bottom are not repeated, which is what
                    // makes an up-down pattern sound even.
                    v.extend(
                        pitches
                            .iter()
                            .rev()
                            .skip(1)
                            .take(pitches.len().saturating_sub(2))
                            .copied(),
                    );
                    v
                }
                ArpMode::AsPlayed => as_played.clone(),
            };
            for pitch in round {
                if let Some(p) = clamp_pitch(pitch as i32 + lift) {
                    order.push(p);
                }
            }
        }
        if order.is_empty() {
            continue;
        }

        let template = group[0];
        let mut tick = start;
        let mut step_index = 0usize;
        while tick < end {
            let pitch = order[step_index % order.len()];
            let remaining = end - tick;
            let length = ((step as f32 * gate) as u64).clamp(1, remaining.max(1));
            out.push(MidiNote {
                start_tick: tick,
                duration_ticks: length,
                pitch,
                velocity: template.velocity,
                channel: template.channel,
                muted: false,
                pan: template.pan,
                fine_cents: template.fine_cents,
                release_velocity: template.release_velocity,
            });
            tick += step;
            step_index += 1;
        }
    }
    out
}

/// Run a note list through the chain, in order.
///
/// Nothing here touches the clip: the caller hands over a copy of the
/// notes and gets back what the instrument should hear.
pub fn apply_chain(notes: &[MidiNote], chain: &[MidiFx]) -> Vec<MidiNote> {
    let mut current: Vec<MidiNote> = notes.to_vec();
    for effect in chain {
        match effect {
            MidiFx::Arpeggiator {
                step_ticks,
                mode,
                gate,
                octaves,
            } => {
                current = arpeggiate(&current, *step_ticks, *mode, *gate, *octaves);
            }
            MidiFx::Chord { intervals } => {
                if intervals.is_empty() {
                    continue;
                }
                let mut grown: Vec<MidiNote> =
                    Vec::with_capacity(current.len() * (intervals.len() + 1));
                for note in &current {
                    grown.push(note.clone());
                    for interval in intervals {
                        let Some(pitch) = clamp_pitch(note.pitch as i32 + *interval as i32) else {
                            continue;
                        };
                        let mut copy = note.clone();
                        copy.pitch = pitch;
                        grown.push(copy);
                    }
                }
                current = grown;
            }
            MidiFx::Scale { root, kind } => {
                for note in current.iter_mut() {
                    note.pitch = snap_to_scale(note.pitch, *root, *kind);
                }
            }
            MidiFx::Transpose { semitones } => {
                current.retain_mut(|note| {
                    match clamp_pitch(note.pitch as i32 + *semitones as i32) {
                        Some(pitch) => {
                            note.pitch = pitch;
                            true
                        }
                        // A note pushed past the end of the keyboard is
                        // dropped rather than folded back into a wrong
                        // octave.
                        None => false,
                    }
                });
            }
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: u64, duration: u64, pitch: u8) -> MidiNote {
        MidiNote {
            start_tick: start,
            duration_ticks: duration,
            pitch,
            ..Default::default()
        }
    }

    #[test]
    fn a_chord_effect_adds_a_note_for_each_interval() {
        let out = apply_chain(
            &[note(0, 480, 60)],
            &[MidiFx::Chord {
                intervals: vec![3, 7],
            }],
        );
        let mut pitches: Vec<u8> = out.iter().map(|n| n.pitch).collect();
        pitches.sort_unstable();
        assert_eq!(pitches, vec![60, 63, 67]);
    }

    #[test]
    fn a_note_pushed_off_the_keyboard_is_dropped_not_folded() {
        let out = apply_chain(&[note(0, 480, 125)], &[MidiFx::Transpose { semitones: 12 }]);
        assert!(out.is_empty(), "127 is the top of the keyboard");
    }

    #[test]
    fn a_scale_moves_a_note_to_the_nearest_one_that_belongs() {
        // C# is not in C major; C is a semitone away and B is a semitone
        // away too, and a tie goes down.
        assert_eq!(snap_to_scale(61, 0, ScaleKind::Major), 60);
        // D is already in the scale.
        assert_eq!(snap_to_scale(62, 0, ScaleKind::Major), 62);
        // F# in E phrygian dominant belongs: root 4, degree 1 is F.
        assert_eq!(snap_to_scale(65, 4, ScaleKind::PhrygianDominant), 65);
    }

    #[test]
    fn an_arpeggiator_turns_one_chord_into_a_run_of_single_notes() {
        let chord = vec![note(0, 1920, 60), note(0, 1920, 64), note(0, 1920, 67)];
        let out = apply_chain(
            &chord,
            &[MidiFx::Arpeggiator {
                step_ticks: 240,
                mode: ArpMode::Up,
                gate: 0.9,
                octaves: 1,
            }],
        );
        assert_eq!(out.len(), 8, "1920 ticks at 240 per step");
        assert_eq!(out[0].pitch, 60);
        assert_eq!(out[1].pitch, 64);
        assert_eq!(out[2].pitch, 67);
        assert_eq!(out[3].pitch, 60, "the pattern repeats");
        assert_eq!(out[1].start_tick, 240);
        assert!(
            out[0].duration_ticks <= 240,
            "a step cannot overlap the next"
        );
    }

    #[test]
    fn an_arpeggiator_going_down_starts_at_the_top() {
        let chord = vec![note(0, 960, 60), note(0, 960, 67)];
        let out = apply_chain(
            &chord,
            &[MidiFx::Arpeggiator {
                step_ticks: 480,
                mode: ArpMode::Down,
                gate: 1.0,
                octaves: 1,
            }],
        );
        assert_eq!(out[0].pitch, 67);
        assert_eq!(out[1].pitch, 60);
    }

    #[test]
    fn two_octaves_climb_before_the_pattern_repeats() {
        let chord = vec![note(0, 1920, 60), note(0, 1920, 64)];
        let out = apply_chain(
            &chord,
            &[MidiFx::Arpeggiator {
                step_ticks: 480,
                mode: ArpMode::Up,
                gate: 1.0,
                octaves: 2,
            }],
        );
        let pitches: Vec<u8> = out.iter().map(|n| n.pitch).collect();
        assert_eq!(pitches, vec![60, 64, 72, 76]);
    }

    #[test]
    fn the_chain_runs_in_order() {
        // Chord first, then transpose: both notes move.
        let out = apply_chain(
            &[note(0, 480, 60)],
            &[
                MidiFx::Chord { intervals: vec![7] },
                MidiFx::Transpose { semitones: 1 },
            ],
        );
        let mut pitches: Vec<u8> = out.iter().map(|n| n.pitch).collect();
        pitches.sort_unstable();
        assert_eq!(pitches, vec![61, 68]);
    }

    #[test]
    fn an_empty_chain_changes_nothing() {
        let notes = vec![note(0, 480, 60), note(480, 480, 62)];
        assert_eq!(apply_chain(&notes, &[]), notes);
    }
}
