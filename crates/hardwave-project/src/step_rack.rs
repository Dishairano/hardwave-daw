//! The channel rack's step patterns, as the engine plays them.
//!
//! The rack's state is the UI's own JSON (`Project::channel_rack_state`),
//! kept opaque so the UI can change its shape. It used to be only saved and
//! loaded: nothing read the steps, so the step sequencer made no sound at
//! all. This reads the parts that sound (the active pattern's steps and
//! their graphs, the swing and the pattern length) into notes per channel.

use hardwave_midi::PPQ;
use serde::Deserialize;
use std::collections::HashMap;

/// One step is a sixteenth note.
pub const TICKS_PER_STEP: u64 = PPQ / 4;
/// The C a step plays when its Note graph is untouched, as in FL.
pub const STEP_BASE_PITCH: i32 = 60;
const DEFAULT_STEPS: u32 = 16;

type Rows = HashMap<String, Vec<f64>>;

#[derive(Debug, Clone, Default, Deserialize)]
struct PatternJson {
    id: String,
    /// This pattern's own length in steps, when set.
    #[serde(default)]
    length: Option<u32>,
    #[serde(default)]
    steps: Rows,
    #[serde(default, rename = "panSteps")]
    pan: Rows,
    /// "Fine pitch": ±12 across the graph, played as ±100 cents.
    #[serde(default, rename = "pitchSteps")]
    fine: Rows,
    /// "Shift": how much of the step the note lasts, 0..1.
    #[serde(default, rename = "gateSteps")]
    gate: Rows,
    /// "Note": semitones from the channel's C.
    #[serde(default, rename = "noteSteps")]
    note: Rows,
    #[serde(default, rename = "releaseSteps")]
    release: Rows,
    /// "Rep": how many times the step repeats inside itself (a ratchet).
    #[serde(default, rename = "repSteps")]
    rep: Rows,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RackJson {
    #[serde(default)]
    patterns: Vec<PatternJson>,
    #[serde(default, rename = "activeId")]
    active_id: Option<String>,
    /// Global swing 0..1.
    #[serde(default)]
    swing: f64,
    /// Each channel's share of the swing, 0..1 (1 when absent).
    #[serde(default)]
    swingmix: HashMap<String, f64>,
    /// Pattern length in steps, when the user set one.
    #[serde(default)]
    length: Option<u32>,
}

/// A pattern with no length of its own lasts whole bars, up to its last step.
fn auto_length(p: &PatternJson) -> u32 {
    let last = p
        .steps
        .values()
        .filter_map(|row| row.iter().rposition(|v| *v > 0.0))
        .max()
        .map(|i| i as u32 + 1)
        .unwrap_or(0);
    DEFAULT_STEPS.max(last.div_ceil(DEFAULT_STEPS) * DEFAULT_STEPS)
}

/// The rack, ready to turn into notes.
#[derive(Debug, Clone, Default)]
pub struct StepRack {
    active: Option<PatternJson>,
    swing: f64,
    swingmix: HashMap<String, f64>,
    length_steps: u32,
}

/// One note the step sequencer plays, in ticks from the pattern's start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepNote {
    pub tick: u64,
    pub duration_ticks: u64,
    pub pitch: u8,
    pub velocity: f32,
    pub pan: f32,
    pub fine_cents: f32,
    pub release_velocity: f32,
}

impl StepRack {
    /// Read the UI's rack JSON. `None` when it is not a rack.
    pub fn parse(json: &str) -> Option<Self> {
        let rack: RackJson = serde_json::from_str(json).ok()?;
        let active = rack
            .active_id
            .as_ref()
            .and_then(|id| rack.patterns.iter().find(|p| &p.id == id))
            .or_else(|| rack.patterns.first())
            .cloned();
        let length_steps = rack
            .length
            .filter(|n| *n > 0)
            .or_else(|| active.as_ref().and_then(|p| p.length).filter(|n| *n > 0))
            .unwrap_or_else(|| active.as_ref().map(auto_length).unwrap_or(DEFAULT_STEPS))
            .min(512);
        Some(Self {
            active,
            swing: rack.swing.clamp(0.0, 1.0),
            swingmix: rack.swingmix,
            length_steps,
        })
    }

    /// How long the pattern is, in ticks: what pattern mode loops.
    pub fn length_ticks(&self) -> u64 {
        self.length_steps as u64 * TICKS_PER_STEP
    }

    /// The notes one channel plays in the active pattern, in time order.
    pub fn notes_for(&self, channel_id: &str) -> Vec<StepNote> {
        let Some(p) = &self.active else {
            return Vec::new();
        };
        let Some(row) = p.steps.get(channel_id) else {
            return Vec::new();
        };
        let at = |rows: &Rows, i: usize, d: f64| {
            rows.get(channel_id)
                .and_then(|r| r.get(i))
                .copied()
                .filter(|v| v.is_finite())
                .unwrap_or(d)
        };
        let mix = self
            .swingmix
            .get(channel_id)
            .copied()
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        // Full swing puts every second sixteenth a third of a step late,
        // the triplet feel.
        let swing_ticks = (self.swing * mix * TICKS_PER_STEP as f64 / 3.0).round() as u64;
        let mut out = Vec::new();
        for (i, &vel) in row.iter().enumerate().take(self.length_steps as usize) {
            if vel.is_nan() || vel <= 0.0 {
                continue;
            }
            let start = i as u64 * TICKS_PER_STEP + if i % 2 == 1 { swing_ticks } else { 0 };
            let pitch = (STEP_BASE_PITCH + at(&p.note, i, 0.0).round() as i32).clamp(0, 127) as u8;
            let gate = at(&p.gate, i, 1.0).clamp(0.05, 1.0);
            let reps = at(&p.rep, i, 1.0).round().clamp(1.0, 8.0) as u64;
            let slice = TICKS_PER_STEP / reps;
            for r in 0..reps {
                out.push(StepNote {
                    tick: start + r * slice,
                    duration_ticks: ((slice as f64 * gate).round() as u64).max(1),
                    pitch,
                    velocity: vel.clamp(0.0, 1.0) as f32,
                    pan: at(&p.pan, i, 0.0).clamp(-1.0, 1.0) as f32,
                    fine_cents: (at(&p.fine, i, 0.0).clamp(-12.0, 12.0) * 100.0 / 12.0) as f32,
                    release_velocity: at(&p.release, i, 1.0).clamp(0.0, 1.0) as f32,
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rack(json: &str) -> StepRack {
        StepRack::parse(json).expect("a rack")
    }

    #[test]
    fn steps_become_sixteenth_notes_on_c() {
        let r = rack(
            r#"{"v":1,"activeId":"p","patterns":[{"id":"p","steps":{"kick":[1,0,0,0,0.5]}}]}"#,
        );
        let n = r.notes_for("kick");
        assert_eq!(n.len(), 2);
        assert_eq!((n[0].tick, n[0].pitch, n[0].velocity), (0, 60, 1.0));
        assert_eq!((n[1].tick, n[1].velocity), (4 * TICKS_PER_STEP, 0.5));
        assert_eq!(n[0].duration_ticks, TICKS_PER_STEP);
        assert!(r.notes_for("snare").is_empty());
        assert_eq!(r.length_ticks(), 16 * TICKS_PER_STEP);
    }

    #[test]
    fn swing_delays_every_second_step_by_its_share() {
        let json = r#"{"activeId":"p","swing":1,"swingmix":{"hat":0.5},
            "patterns":[{"id":"p","steps":{"hat":[1,1,1,1],"kick":[1,1]}}]}"#;
        let r = rack(json);
        let full = TICKS_PER_STEP / 3;
        let k = r.notes_for("kick");
        assert_eq!(k[1].tick, TICKS_PER_STEP + full);
        let h = r.notes_for("hat");
        assert_eq!(h[0].tick, 0);
        assert_eq!(
            h[1].tick,
            TICKS_PER_STEP + (full as f64 / 2.0).round() as u64
        );
        assert_eq!(h[2].tick, 2 * TICKS_PER_STEP, "odd steps stay on the grid");
    }

    #[test]
    fn graphs_set_pitch_pan_length_and_repeats() {
        let json = r#"{"activeId":"p","patterns":[{"id":"p",
            "steps":{"c":[1]},"noteSteps":{"c":[7]},"panSteps":{"c":[-1]},
            "gateSteps":{"c":[0.5]},"repSteps":{"c":[4]}}]}"#;
        let n = rack(json).notes_for("c");
        assert_eq!(n.len(), 4, "a ratchet of four");
        assert_eq!(n[0].pitch, 67);
        assert_eq!(n[0].pan, -1.0);
        assert_eq!(n[1].tick, TICKS_PER_STEP / 4);
        assert_eq!(n[0].duration_ticks, TICKS_PER_STEP / 8);
    }

    #[test]
    fn a_pattern_lasts_whole_bars_unless_it_says_otherwise() {
        let r = rack(
            r#"{"activeId":"p","patterns":[{"id":"p","steps":{"c":[0,0,0,0,0,0,0,0,0,0,0,0,1]}}]}"#,
        );
        assert_eq!(
            r.length_ticks(),
            16 * TICKS_PER_STEP,
            "13 steps last the bar"
        );
        let long = rack(
            r#"{"activeId":"p","patterns":[{"id":"p","steps":{"c":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]}}]}"#,
        );
        assert_eq!(long.length_ticks(), 32 * TICKS_PER_STEP);
        let own = rack(r#"{"activeId":"p","patterns":[{"id":"p","length":12,"steps":{"c":[1]}}]}"#);
        assert_eq!(own.length_ticks(), 12 * TICKS_PER_STEP);
    }

    #[test]
    fn the_active_pattern_and_length_are_honoured() {
        let json = r#"{"activeId":"b","length":8,"patterns":[
            {"id":"a","steps":{"c":[1]}},
            {"id":"b","steps":{"c":[0,0,0,0,0,0,0,0,1]}}]}"#;
        let r = rack(json);
        assert!(
            r.notes_for("c").is_empty(),
            "step 9 is past an 8-step pattern"
        );
        assert_eq!(r.length_ticks(), 8 * TICKS_PER_STEP);
        assert!(StepRack::parse("not json").is_none());
    }
}
