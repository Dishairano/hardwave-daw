//! Grooves: take the timing and accent of one part and put it on another.
//!
//! Quantizing to the grid makes a part correct and lifeless. A groove is the
//! opposite move: it takes what a played part does against the grid, how
//! early or late each step lands and how hard it is hit, and applies that to
//! a part that was typed in.
//!
//! The DAW had quantize with a swing amount, which is one fixed groove. This
//! is any groove you already have in the song.

use crate::MidiNote;
use serde::{Deserialize, Serialize};

/// The timing and accent of one bar-length pattern, as offsets from a grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Groove {
    pub name: String,
    /// Grid the groove was measured against, in ticks (240 = a sixteenth at
    /// 960 PPQ).
    pub grid_ticks: u64,
    /// How long the pattern is before it repeats, in ticks.
    pub length_ticks: u64,
    /// One entry per grid step that had a note: how far off the grid it sat,
    /// and how hard it was hit relative to the part's average.
    pub steps: Vec<GrooveStep>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GrooveStep {
    /// Which grid step, from the start of the pattern.
    pub step: u64,
    /// Ticks early (negative) or late (positive).
    pub offset_ticks: i64,
    /// Velocity relative to the part's average, as a multiplier.
    pub velocity_ratio: f32,
}

/// Read the groove out of a part.
///
/// Notes are matched to their nearest grid line, and what is recorded is how
/// far each sat from it. A step with several notes (a chord) is recorded
/// once, from the first of them, because a chord is one event played once.
pub fn extract(notes: &[MidiNote], grid_ticks: u64, length_ticks: u64, name: String) -> Groove {
    let grid = grid_ticks.max(1);
    let length = length_ticks.max(grid);
    let mut steps: Vec<GrooveStep> = Vec::new();
    let average_velocity = if notes.is_empty() {
        1.0
    } else {
        notes.iter().map(|n| n.velocity).sum::<f32>() / notes.len() as f32
    };

    for note in notes {
        let within = note.start_tick % length;
        let step = (within as f64 / grid as f64).round() as u64;
        let grid_tick = step * grid;
        let offset = within as i64 - grid_tick as i64;
        // Half a step away is not "off the grid", it is a different step.
        if offset.unsigned_abs() > grid / 2 {
            continue;
        }
        if steps.iter().any(|s| s.step == step) {
            continue;
        }
        steps.push(GrooveStep {
            step,
            offset_ticks: offset,
            velocity_ratio: if average_velocity > 0.0 {
                note.velocity / average_velocity
            } else {
                1.0
            },
        });
    }
    steps.sort_by_key(|s| s.step);
    Groove {
        name,
        grid_ticks: grid,
        length_ticks: length,
        steps,
    }
}

/// Put a groove on a part.
///
/// `strength` from 0 to 1 says how far towards the groove to move: 0 leaves
/// the part alone, 1 places it exactly as the groove was played. Notes on
/// steps the groove has nothing for are quantized to the grid and left
/// there, because inventing a feel for them would be making something up.
pub fn apply(notes: &mut [MidiNote], groove: &Groove, strength: f32) {
    let strength = strength.clamp(0.0, 1.0);
    if strength == 0.0 || groove.steps.is_empty() {
        return;
    }
    let grid = groove.grid_ticks.max(1);
    let length = groove.length_ticks.max(grid);
    for note in notes.iter_mut() {
        let bar = note.start_tick / length * length;
        let within = note.start_tick % length;
        let step = (within as f64 / grid as f64).round() as u64;
        let grid_tick = step * grid;
        let wanted_offset = groove
            .steps
            .iter()
            .find(|s| s.step == step % (length / grid).max(1))
            .map(|s| s.offset_ticks)
            .unwrap_or(0);
        let current_offset = within as i64 - grid_tick as i64;
        let moved = current_offset as f32 + (wanted_offset - current_offset) as f32 * strength;
        let new_within = grid_tick as i64 + moved.round() as i64;
        note.start_tick = (bar as i64 + new_within).max(0) as u64;

        if let Some(s) = groove
            .steps
            .iter()
            .find(|s| s.step == step % (length / grid).max(1))
        {
            let target = (note.velocity * s.velocity_ratio).clamp(0.0, 1.0);
            note.velocity = note.velocity + (target - note.velocity) * strength;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: u64, velocity: f32) -> MidiNote {
        MidiNote {
            start_tick: start,
            duration_ticks: 120,
            pitch: 60,
            velocity,
            channel: 0,
            muted: false,
        }
    }

    #[test]
    fn a_played_part_gives_up_its_timing_and_accents() {
        // Sixteenths at 960 PPQ: 0, 240, 480, 720. The off-beats are late
        // and harder, which is what swing is.
        let notes = vec![note(0, 0.8), note(260, 1.0), note(480, 0.8), note(742, 1.0)];
        let groove = extract(&notes, 240, 960, "swing".into());
        assert_eq!(groove.steps.len(), 4);
        assert_eq!(groove.steps[0].offset_ticks, 0);
        assert_eq!(groove.steps[1].offset_ticks, 20);
        assert_eq!(groove.steps[3].offset_ticks, 22);
        assert!(
            groove.steps[1].velocity_ratio > 1.0,
            "the off-beat is harder"
        );
    }

    #[test]
    fn a_typed_part_takes_the_groove_on() {
        let source = vec![note(0, 0.8), note(260, 1.0), note(480, 0.8), note(742, 1.0)];
        let groove = extract(&source, 240, 960, "swing".into());

        let mut typed = vec![note(0, 0.9), note(240, 0.9), note(480, 0.9), note(720, 0.9)];
        apply(&mut typed, &groove, 1.0);
        assert_eq!(typed[1].start_tick, 260, "the off-beat moves late");
        assert_eq!(typed[3].start_tick, 742);
        assert!(typed[1].velocity > typed[0].velocity, "and harder");
    }

    #[test]
    fn half_strength_moves_half_way() {
        let source = vec![note(0, 0.8), note(260, 0.8)];
        let groove = extract(&source, 240, 480, "half".into());
        let mut typed = vec![note(0, 0.8), note(240, 0.8)];
        apply(&mut typed, &groove, 0.5);
        assert_eq!(typed[1].start_tick, 250);
    }

    #[test]
    fn a_groove_with_nothing_in_it_leaves_the_part_alone() {
        let groove = Groove {
            name: "empty".into(),
            grid_ticks: 240,
            length_ticks: 960,
            steps: Vec::new(),
        };
        let mut typed = vec![note(37, 0.5)];
        apply(&mut typed, &groove, 1.0);
        assert_eq!(typed[0].start_tick, 37, "nothing to apply, nothing moved");
    }
}
