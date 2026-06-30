//! Piano-roll note generators & transforms (FL Studio-style): arpeggiator,
//! strum, and scale snapping.
//!
//! Every function here is a pure transform over note lists — no engine,
//! no I/O — so they unit-test cleanly and can be driven from either a
//! Tauri command or an offline tool. The piano-roll UI calls these on the
//! current selection; the result replaces (arp) or mutates (strum,
//! scale-snap) the selected notes.

use crate::{GridDivision, MidiNote};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Arpeggiator
// ---------------------------------------------------------------------------

/// Order in which the chord's pitches are stepped through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArpDirection {
    Up,
    Down,
    UpDown,
    DownUp,
    /// Preserve the order the notes were played/entered in.
    AsPlayed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ArpSettings {
    pub direction: ArpDirection,
    /// Step length (and grid) for each arpeggiated note.
    pub rate: GridDivision,
    /// Octave span: 1 = chord as entered, 2 repeats it one octave up, etc.
    pub octaves: u8,
    /// Fraction of the step the note sounds for (`0.05..=1.0`). 1.0 is legato.
    pub gate: f32,
}

impl Default for ArpSettings {
    fn default() -> Self {
        Self {
            direction: ArpDirection::Up,
            rate: GridDivision::Sixteenth,
            octaves: 1,
            gate: 0.9,
        }
    }
}

/// Turn a chord (the selected notes) into an arpeggio that fills the
/// selection's time span. Returns a fresh note list to replace the input.
///
/// The time span runs from the earliest `start_tick` to the latest note
/// end. Pitches are taken from the unique input pitches (carrying each
/// pitch's source velocity), expanded across `octaves`, then ordered by
/// `direction`. Steps of `rate` are laid down across the span, cycling
/// the pitch sequence; each step sounds for `gate` of the step length.
pub fn arpeggiate(notes: &[MidiNote], settings: &ArpSettings) -> Vec<MidiNote> {
    let live: Vec<&MidiNote> = notes.iter().filter(|n| !n.muted).collect();
    if live.is_empty() {
        return Vec::new();
    }

    let span_start = live.iter().map(|n| n.start_tick).min().unwrap();
    let span_end = live
        .iter()
        .map(|n| n.start_tick + n.duration_ticks)
        .max()
        .unwrap();
    if span_end <= span_start {
        return Vec::new();
    }

    let sequence = build_arp_sequence(&live, settings);
    if sequence.is_empty() {
        return Vec::new();
    }

    let step = settings.rate.ticks().max(1);
    let gate = settings.gate.clamp(0.05, 1.0);
    let gate_len = ((step as f64 * gate as f64).round() as u64).max(1);
    let channel = live[0].channel;

    let mut out = Vec::new();
    let mut tick = span_start;
    let mut idx = 0usize;
    while tick < span_end {
        let (pitch, velocity) = sequence[idx % sequence.len()];
        // Clamp the final note so it never spills past the selection.
        let dur = gate_len.min(span_end - tick).max(1);
        out.push(MidiNote {
            start_tick: tick,
            duration_ticks: dur,
            pitch,
            velocity,
            channel,
            muted: false,
        });
        tick += step;
        idx += 1;
    }
    out
}

/// Build the ordered `(pitch, velocity)` sequence for an arpeggio:
/// unique pitches, octave-expanded, then ordered per direction.
fn build_arp_sequence(live: &[&MidiNote], settings: &ArpSettings) -> Vec<(u8, f32)> {
    let octaves = settings.octaves.max(1);

    // For AsPlayed we keep entry order (start_tick, then pitch); for every
    // other direction we work from the ascending unique-pitch set.
    let base: Vec<(u8, f32)> = if settings.direction == ArpDirection::AsPlayed {
        let mut ordered: Vec<&&MidiNote> = live.iter().collect();
        ordered.sort_by_key(|n| (n.start_tick, n.pitch));
        dedup_pitches(ordered.into_iter().map(|n| (n.pitch, n.velocity)))
    } else {
        let mut asc: Vec<(u8, f32)> = dedup_pitches(live.iter().map(|n| (n.pitch, n.velocity)));
        asc.sort_by_key(|(p, _)| *p);
        asc
    };

    // Octave expansion: stack the base set up by 12 semitones per octave,
    // dropping anything that would exceed MIDI 127.
    let mut expanded: Vec<(u8, f32)> = Vec::new();
    for o in 0..octaves {
        let shift = 12u16 * o as u16;
        for &(p, v) in &base {
            let np = p as u16 + shift;
            if np <= 127 {
                expanded.push((np as u8, v));
            }
        }
    }
    if expanded.len() <= 1 {
        return expanded;
    }

    match settings.direction {
        ArpDirection::Up | ArpDirection::AsPlayed => expanded,
        ArpDirection::Down => {
            expanded.reverse();
            expanded
        }
        ArpDirection::UpDown => {
            // Up then back down, without repeating the top/bottom note.
            let mut seq = expanded.clone();
            for item in expanded
                .iter()
                .rev()
                .skip(1)
                .take(expanded.len().saturating_sub(2))
            {
                seq.push(*item);
            }
            seq
        }
        ArpDirection::DownUp => {
            let mut seq: Vec<(u8, f32)> = expanded.iter().rev().copied().collect();
            for item in expanded
                .iter()
                .skip(1)
                .take(expanded.len().saturating_sub(2))
            {
                seq.push(*item);
            }
            seq
        }
    }
}

/// Keep the first velocity seen per pitch, preserving iteration order.
fn dedup_pitches(iter: impl Iterator<Item = (u8, f32)>) -> Vec<(u8, f32)> {
    let mut seen = [false; 128];
    let mut out = Vec::new();
    for (p, v) in iter {
        if p < 128 && !seen[p as usize] {
            seen[p as usize] = true;
            out.push((p, v));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Strum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrumDirection {
    /// Low notes first (guitar down-strum).
    Up,
    /// High notes first (guitar up-strum).
    Down,
}

/// Spread a chord's note onsets in time so it strums instead of hitting
/// at once. Treats the whole slice as one chord (the piano-roll passes
/// the current selection). Notes are ranked by pitch per `direction` and
/// delayed `rank * spread_ticks`; durations are preserved.
pub fn strum(notes: &mut [MidiNote], spread_ticks: u64, direction: StrumDirection) {
    if spread_ticks == 0 || notes.len() < 2 {
        return;
    }
    // Rank by pitch. `idx_by_rank[r]` = index of the note in rank position r.
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by_key(|&i| notes[i].pitch);
    if direction == StrumDirection::Down {
        order.reverse();
    }
    for (rank, &i) in order.iter().enumerate() {
        notes[i].start_tick += rank as u64 * spread_ticks;
    }
}

// ---------------------------------------------------------------------------
// Scale snapping
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    Major,
    NaturalMinor,
    HarmonicMinor,
    MelodicMinor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    PentatonicMajor,
    PentatonicMinor,
    Blues,
    WholeTone,
    HungarianMinor,
    Chromatic,
}

impl Scale {
    /// Semitone offsets (from the root) that belong to the scale.
    pub fn intervals(&self) -> &'static [u8] {
        match self {
            Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
            Scale::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
            Scale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Scale::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
            Scale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Scale::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Scale::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            Scale::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Scale::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            Scale::PentatonicMajor => &[0, 2, 4, 7, 9],
            Scale::PentatonicMinor => &[0, 3, 5, 7, 10],
            Scale::Blues => &[0, 3, 5, 6, 7, 10],
            Scale::WholeTone => &[0, 2, 4, 6, 8, 10],
            Scale::HungarianMinor => &[0, 2, 3, 6, 7, 8, 11],
            Scale::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        }
    }
}

/// Snap every note's pitch to the nearest pitch in `scale` rooted at
/// `root` (0 = C … 11 = B). On an exact tie the lower pitch wins (matching
/// FL's bias). Chromatic is a no-op. Pitches are clamped to MIDI 0..=127.
pub fn snap_to_scale(notes: &mut [MidiNote], root: u8, scale: Scale) {
    if scale == Scale::Chromatic {
        return;
    }
    let degrees = scale.intervals();
    let root = (root % 12) as i32;
    for note in notes.iter_mut() {
        note.pitch = nearest_in_scale(note.pitch, root, degrees);
    }
}

fn nearest_in_scale(pitch: u8, root: i32, degrees: &[u8]) -> u8 {
    let p = pitch as i32;
    let rel = (p - root).rem_euclid(12);
    // Distance to the nearest degree, considering moving up or down. We
    // search every degree both directions and keep the smallest move;
    // ties resolve downward (delta_down checked first).
    let mut best_delta = i32::MAX;
    for &deg in degrees {
        let deg = deg as i32;
        let down = (rel - deg).rem_euclid(12); // semitones to move DOWN
        let up = (deg - rel).rem_euclid(12); // semitones to move UP
                                             // Candidate moves: -down and +up.
        for cand in [-down, up] {
            if cand.abs() < best_delta.abs()
                || (cand.abs() == best_delta.abs() && cand < best_delta)
            {
                best_delta = cand;
            }
        }
    }
    if best_delta == i32::MAX {
        return pitch;
    }
    (p + best_delta).clamp(0, 127) as u8
}

// ---------------------------------------------------------------------------
// Note repeat / chop
// ---------------------------------------------------------------------------

/// Replace each note with `repeats` evenly-spaced retriggers of the same
/// pitch across its original duration (hi-hat rolls, stutters, snare
/// rushes). `gate` (0.05..=1.0) sets how much of each slice sounds.
/// `repeats <= 1` returns the notes unchanged. Velocity + pitch carry; a
/// `decay` of `d` scales each successive hit by `(1-d)` for natural rolls.
pub fn note_repeat(notes: &[MidiNote], repeats: u32, gate: f32, decay: f32) -> Vec<MidiNote> {
    if repeats <= 1 {
        return notes.to_vec();
    }
    let gate = gate.clamp(0.05, 1.0);
    let decay = decay.clamp(0.0, 1.0);
    let mut out = Vec::with_capacity(notes.len() * repeats as usize);
    for n in notes {
        if n.muted || n.duration_ticks == 0 {
            out.push(n.clone());
            continue;
        }
        let slice = (n.duration_ticks / repeats as u64).max(1);
        let hit_len = ((slice as f64 * gate as f64).round() as u64).max(1);
        for r in 0..repeats {
            let vel = (n.velocity * (1.0 - decay).powi(r as i32)).clamp(0.0, 1.0);
            out.push(MidiNote {
                start_tick: n.start_tick + r as u64 * slice,
                duration_ticks: hit_len,
                pitch: n.pitch,
                velocity: vel,
                channel: n.channel,
                muted: false,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Humanize
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HumanizeSettings {
    /// Max random start-tick offset (± this many ticks).
    pub timing_ticks: u64,
    /// Max random velocity deviation as a fraction (0.0..=1.0).
    pub velocity_amount: f32,
    /// Seed for reproducibility (same seed → same humanization).
    pub seed: u64,
}

impl Default for HumanizeSettings {
    fn default() -> Self {
        Self {
            timing_ticks: 20,
            velocity_amount: 0.15,
            seed: 0x5DEE_CE66,
        }
    }
}

/// Nudge each selected note's start and velocity by a small random amount
/// so a programmed part feels less mechanical. Deterministic for a given
/// seed. Start ticks never go negative; velocities stay in `[0, 1]`.
pub fn humanize(notes: &mut [MidiNote], settings: &HumanizeSettings) {
    let mut state = settings.seed | 1;
    // xorshift64* → bipolar [-1, 1).
    let mut bipolar = || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let u = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
        u * 2.0 - 1.0
    };
    let vamt = settings.velocity_amount.clamp(0.0, 1.0);
    for n in notes.iter_mut() {
        if n.muted {
            continue;
        }
        if settings.timing_ticks > 0 {
            let off = (bipolar() * settings.timing_ticks as f64).round() as i64;
            n.start_tick = (n.start_tick as i64 + off).max(0) as u64;
        }
        if vamt > 0.0 {
            let dev = bipolar() as f32 * vamt;
            n.velocity = (n.velocity * (1.0 + dev)).clamp(0.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: u64, dur: u64, pitch: u8) -> MidiNote {
        MidiNote {
            start_tick: start,
            duration_ticks: dur,
            pitch,
            velocity: 0.8,
            channel: 0,
            muted: false,
        }
    }

    #[test]
    fn arp_up_cycles_pitches_across_span() {
        // C-E-G held for one bar (3840 ticks), 1/16 steps = 240 ticks → 16 steps.
        let chord = vec![note(0, 3840, 60), note(0, 3840, 64), note(0, 3840, 67)];
        let s = ArpSettings {
            direction: ArpDirection::Up,
            rate: GridDivision::Sixteenth,
            octaves: 1,
            gate: 1.0,
        };
        let out = arpeggiate(&chord, &s);
        assert_eq!(out.len(), 16, "one bar of 1/16 steps");
        // Ascending cycle C,E,G,C,E,G...
        assert_eq!(out[0].pitch, 60);
        assert_eq!(out[1].pitch, 64);
        assert_eq!(out[2].pitch, 67);
        assert_eq!(out[3].pitch, 60);
        // Steps are grid-aligned.
        assert_eq!(out[1].start_tick, 240);
        assert_eq!(out[4].start_tick, 960);
    }

    #[test]
    fn arp_gate_shortens_notes() {
        let chord = vec![note(0, 960, 60), note(0, 960, 64)];
        let s = ArpSettings {
            direction: ArpDirection::Up,
            rate: GridDivision::Sixteenth, // 240-tick step
            octaves: 1,
            gate: 0.5,
        };
        let out = arpeggiate(&chord, &s);
        assert!(out.iter().all(|n| n.duration_ticks == 120));
    }

    #[test]
    fn arp_octaves_expand_upward() {
        let chord = vec![note(0, 480, 60)];
        let s = ArpSettings {
            direction: ArpDirection::Up,
            rate: GridDivision::Eighth, // 480
            octaves: 2,
            gate: 1.0,
        };
        // Span is 480 ticks → exactly one step, but the sequence must hold
        // both octaves so a longer span would alternate 60, 72.
        let seq = build_arp_sequence(&chord.iter().collect::<Vec<_>>(), &s);
        assert_eq!(
            seq.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
            vec![60, 72]
        );
        let _ = arpeggiate(&chord, &s);
    }

    #[test]
    fn arp_updown_no_repeated_endpoints() {
        let chord = vec![note(0, 480, 60), note(0, 480, 64), note(0, 480, 67)];
        let s = ArpSettings {
            direction: ArpDirection::UpDown,
            rate: GridDivision::Quarter,
            octaves: 1,
            gate: 1.0,
        };
        let seq: Vec<u8> = build_arp_sequence(&chord.iter().collect::<Vec<_>>(), &s)
            .iter()
            .map(|(p, _)| *p)
            .collect();
        // Up: 60,64,67 then down without repeating 67 or 60 → 64.
        assert_eq!(seq, vec![60, 64, 67, 64]);
    }

    #[test]
    fn arp_empty_input_is_empty() {
        assert!(arpeggiate(&[], &ArpSettings::default()).is_empty());
    }

    #[test]
    fn strum_up_delays_by_pitch_rank() {
        let mut chord = vec![note(0, 480, 67), note(0, 480, 60), note(0, 480, 64)];
        strum(&mut chord, 30, StrumDirection::Up);
        // Sorted ascending: 60 (rank0,+0), 64 (rank1,+30), 67 (rank2,+60).
        let g60 = chord.iter().find(|n| n.pitch == 60).unwrap();
        let g64 = chord.iter().find(|n| n.pitch == 64).unwrap();
        let g67 = chord.iter().find(|n| n.pitch == 67).unwrap();
        assert_eq!(g60.start_tick, 0);
        assert_eq!(g64.start_tick, 30);
        assert_eq!(g67.start_tick, 60);
    }

    #[test]
    fn strum_down_reverses_order() {
        let mut chord = vec![note(0, 480, 60), note(0, 480, 64), note(0, 480, 67)];
        strum(&mut chord, 30, StrumDirection::Down);
        let g67 = chord.iter().find(|n| n.pitch == 67).unwrap();
        let g60 = chord.iter().find(|n| n.pitch == 60).unwrap();
        assert_eq!(g67.start_tick, 0); // highest first
        assert_eq!(g60.start_tick, 60);
    }

    #[test]
    fn scale_snap_major_pulls_offnotes_in() {
        // C major rooted at C(0). C#(61) → C(60), F#(66) → F(65) or G(67):
        // both are 1 away; tie resolves downward → F(65).
        let mut notes = vec![note(0, 100, 61), note(0, 100, 66), note(0, 100, 60)];
        snap_to_scale(&mut notes, 0, Scale::Major);
        assert_eq!(notes[0].pitch, 60); // C# → C
        assert_eq!(notes[1].pitch, 65); // F# → F (tie, down)
        assert_eq!(notes[2].pitch, 60); // C stays
    }

    #[test]
    fn scale_snap_respects_root() {
        // A natural minor (root A=9) contains B(71); already in-scale, no move.
        let mut notes = vec![note(0, 100, 71)];
        snap_to_scale(&mut notes, 9, Scale::NaturalMinor);
        assert_eq!(notes[0].pitch, 71);
    }

    #[test]
    fn scale_snap_chromatic_is_noop() {
        let mut notes = vec![note(0, 100, 61), note(0, 100, 66)];
        snap_to_scale(&mut notes, 0, Scale::Chromatic);
        assert_eq!(notes[0].pitch, 61);
        assert_eq!(notes[1].pitch, 66);
    }

    #[test]
    fn humanize_is_deterministic_and_bounded() {
        let base = vec![
            note(1000, 100, 60),
            note(2000, 100, 64),
            note(3000, 100, 67),
        ];
        let s = HumanizeSettings {
            timing_ticks: 30,
            velocity_amount: 0.2,
            seed: 7,
        };
        let mut a = base.clone();
        let mut b = base.clone();
        humanize(&mut a, &s);
        humanize(&mut b, &s);
        // Same seed → identical result.
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.start_tick, y.start_tick);
            assert!((x.velocity - y.velocity).abs() < 1e-9);
        }
        // Within bounds: timing within ±30, velocity in [0,1].
        for (orig, h) in base.iter().zip(a.iter()) {
            let delta = h.start_tick as i64 - orig.start_tick as i64;
            assert!(delta.abs() <= 30, "timing offset {delta} exceeds ±30");
            assert!((0.0..=1.0).contains(&h.velocity));
        }
    }

    #[test]
    fn humanize_zero_settings_is_noop() {
        let mut notes = vec![note(500, 100, 60)];
        humanize(
            &mut notes,
            &HumanizeSettings {
                timing_ticks: 0,
                velocity_amount: 0.0,
                seed: 1,
            },
        );
        assert_eq!(notes[0].start_tick, 500);
        assert!((notes[0].velocity - 0.8).abs() < 1e-9);
    }

    #[test]
    fn note_repeat_splits_into_even_hits() {
        // One 400-tick note → 4 hits of 100 ticks, full gate.
        let src = vec![note(0, 400, 42)];
        let out = note_repeat(&src, 4, 1.0, 0.0);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].start_tick, 0);
        assert_eq!(out[1].start_tick, 100);
        assert_eq!(out[3].start_tick, 300);
        assert!(out.iter().all(|n| n.pitch == 42 && n.duration_ticks == 100));
    }

    #[test]
    fn note_repeat_gate_and_decay() {
        let src = vec![note(0, 400, 42)];
        let out = note_repeat(&src, 4, 0.5, 0.5);
        assert!(out.iter().all(|n| n.duration_ticks == 50)); // 100 * 0.5 gate
                                                             // Decay: each hit quieter than the last.
        assert!(out[1].velocity < out[0].velocity);
        assert!(out[3].velocity < out[1].velocity);
    }

    #[test]
    fn note_repeat_one_is_noop() {
        let src = vec![note(0, 400, 42)];
        assert_eq!(note_repeat(&src, 1, 1.0, 0.0).len(), 1);
    }
}
