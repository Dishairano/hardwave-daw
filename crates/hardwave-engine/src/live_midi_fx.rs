//! A track's MIDI effects on what is played live.
//!
//! Clip notes go through the track's MIDI effects when the graph is built.
//! A keyboard played live went through them only on the way to the
//! built-in synth: a plug-in instrument got the keys as they were played,
//! so a chord, scale or transpose effect did nothing on it, and the
//! arpeggiator did nothing live at all. This is the one stage both now
//! read from: the keys in, the notes the effects make of them out, with
//! the arpeggiator stepping in time with the song's tempo from the first
//! key pressed.
//!
//! It runs on the audio thread, so it never allocates: the notes each key
//! became and the keys held are fixed arrays, and the output goes into a
//! buffer the node reserves up front.

use hardwave_midi::midi_fx::{snap_to_scale, ArpMode, MidiFx};
use hardwave_midi::MidiEvent;

/// The most notes one key can become (a chord on a chord is cut off here).
pub const KEY_NOTES_MAX: usize = 16;
/// The most different pitches an arpeggiator walks over, before octaves.
const ARP_PITCHES_MAX: usize = 64;

/// What one key turned into: pitches and how many.
#[derive(Clone, Copy)]
struct KeyNotes {
    pitches: [u8; KEY_NOTES_MAX],
    count: u8,
    velocity: f32,
    channel: u8,
}

impl KeyNotes {
    const EMPTY: Self = Self {
        pitches: [0; KEY_NOTES_MAX],
        count: 0,
        velocity: 0.0,
        channel: 0,
    };

    fn notes(&self) -> &[u8] {
        &self.pitches[..self.count as usize]
    }
}

/// The arpeggiator's settings, taken from the chain.
#[derive(Clone, Copy, PartialEq)]
struct Arp {
    step_ticks: u64,
    mode: ArpMode,
    gate: f32,
    octaves: u8,
}

pub struct LiveMidiFx {
    chain: Vec<MidiFx>,
    /// Where the first arpeggiator sits in the chain: the effects before
    /// it work on each key, the ones after on each step it plays.
    arp_at: Option<usize>,
    /// For each key held, the notes it became (before an arpeggiator).
    keys: [KeyNotes; 128],
    /// Keys held, in the order they went down, for the arpeggiator.
    order: [u8; 128],
    held: usize,
    /// The arpeggiator: which step is next, when (in samples from the
    /// start of the block), and the notes of the step sounding now with
    /// when they end.
    step: usize,
    next_step_at: f64,
    sounding: KeyNotes,
    off_at: Option<f64>,
    /// The chain changed under held keys: let every note go and play the
    /// keys again through the new chain.
    release_all: bool,
}

impl Default for LiveMidiFx {
    fn default() -> Self {
        Self {
            chain: Vec::new(),
            arp_at: None,
            keys: [KeyNotes::EMPTY; 128],
            order: [0; 128],
            held: 0,
            step: 0,
            next_step_at: 0.0,
            sounding: KeyNotes::EMPTY,
            off_at: None,
            release_all: false,
        }
    }
}

/// Every pitch one note becomes through `effects` (an arpeggiator among
/// them is skipped: it is not a per-note effect).
fn apply(effects: &[MidiFx], pitch: u8) -> ([u8; KEY_NOTES_MAX], usize) {
    let mut buffer = [0u8; KEY_NOTES_MAX];
    buffer[0] = pitch;
    let mut count = 1usize;
    for effect in effects {
        match effect {
            MidiFx::Chord { intervals } => {
                let held = count;
                for index in 0..held {
                    let base = buffer[index] as i32;
                    for interval in intervals {
                        if count >= KEY_NOTES_MAX {
                            break;
                        }
                        let moved = base + *interval as i32;
                        if (0..=127).contains(&moved) {
                            buffer[count] = moved as u8;
                            count += 1;
                        }
                    }
                }
            }
            MidiFx::Scale { root, kind } => {
                for slot in buffer.iter_mut().take(count) {
                    *slot = snap_to_scale(*slot, *root, *kind);
                }
            }
            MidiFx::Transpose { semitones } => {
                let mut kept = 0usize;
                for index in 0..count {
                    let moved = buffer[index] as i32 + *semitones as i32;
                    if (0..=127).contains(&moved) {
                        buffer[kept] = moved as u8;
                        kept += 1;
                    }
                }
                count = kept;
            }
            MidiFx::Arpeggiator { .. } => {}
        }
    }
    (buffer, count)
}

fn note_on(timing: u32, channel: u8, note: u8, velocity: f32) -> MidiEvent {
    MidiEvent::NoteOn {
        timing,
        channel,
        note,
        velocity,
    }
}

fn note_off(timing: u32, channel: u8, note: u8) -> MidiEvent {
    MidiEvent::NoteOff {
        timing,
        channel,
        note,
        velocity: 0.0,
    }
}

/// Push without growing: on the audio thread a full buffer drops the
/// rest of the block's notes rather than allocate.
fn emit(out: &mut Vec<MidiEvent>, event: MidiEvent) {
    if out.len() < out.capacity() {
        out.push(event);
    }
}

impl LiveMidiFx {
    /// Whether a key is down or a note is sounding.
    pub fn busy(&self) -> bool {
        self.sounding.count > 0 || self.keys.iter().any(|k| k.count > 0)
    }

    pub fn set_chain(&mut self, chain: Vec<MidiFx>) {
        if chain != self.chain && self.busy() {
            self.release_all = true;
        }
        self.arp_at = chain
            .iter()
            .position(|e| matches!(e, MidiFx::Arpeggiator { .. }));
        self.chain = chain;
    }

    /// Take over what `old` was holding and playing, from the node a graph
    /// rebuild replaces, so a held key is let go properly and an
    /// arpeggio keeps going. This node's own chain stays.
    pub fn take_over(&mut self, old: LiveMidiFx) {
        let release = old.release_all || (old.chain != self.chain && old.busy());
        self.keys = old.keys;
        self.order = old.order;
        self.held = old.held;
        self.step = old.step;
        self.next_step_at = old.next_step_at;
        self.sounding = old.sounding;
        self.off_at = old.off_at;
        self.release_all = release;
    }

    /// Forget the keys and the arpeggio without letting anything go: the
    /// keyboard moved to another track, and the node lets go of the
    /// notes it played itself.
    pub fn forget(&mut self) {
        self.keys = [KeyNotes::EMPTY; 128];
        self.held = 0;
        self.sounding = KeyNotes::EMPTY;
        self.off_at = None;
        self.release_all = false;
    }

    fn arp(&self) -> Option<Arp> {
        match self.arp_at.and_then(|i| self.chain.get(i)) {
            Some(MidiFx::Arpeggiator {
                step_ticks,
                mode,
                gate,
                octaves,
            }) => Some(Arp {
                step_ticks: (*step_ticks).max(1),
                mode: *mode,
                gate: gate.clamp(0.05, 1.0),
                octaves: (*octaves).clamp(1, 4),
            }),
            _ => None,
        }
    }

    fn before_arp(&self) -> &[MidiFx] {
        match self.arp_at {
            Some(i) => &self.chain[..i],
            None => &self.chain,
        }
    }

    fn after_arp(&self) -> &[MidiFx] {
        match self.arp_at {
            Some(i) => &self.chain[i + 1..],
            None => &[],
        }
    }

    /// Turn the keys played this block (`input`) into what the effects
    /// make of them (`out`, cleared first). `block_len` is the block in
    /// samples; `samples_per_tick` follows the song's tempo.
    pub fn process(
        &mut self,
        input: &[MidiEvent],
        out: &mut Vec<MidiEvent>,
        block_len: usize,
        samples_per_tick: f64,
    ) {
        out.clear();
        if self.release_all {
            self.release_all = false;
            self.rekey(out);
        }
        let arp = self.arp();
        let mut at = 0u32;
        for event in input {
            let timing = match *event {
                MidiEvent::NoteOn { timing, .. }
                | MidiEvent::NoteOff { timing, .. }
                | MidiEvent::ControlChange { timing, .. }
                | MidiEvent::PitchBend { timing, .. }
                | MidiEvent::Aftertouch { timing, .. }
                | MidiEvent::ChannelPressure { timing, .. }
                | MidiEvent::NoteExpression { timing, .. } => timing,
            }
            .max(at);
            if let Some(arp) = arp {
                self.run_arp(arp, at as f64, timing as f64, samples_per_tick, out);
            }
            at = timing;
            match *event {
                MidiEvent::NoteOn {
                    note,
                    velocity,
                    channel,
                    ..
                } if velocity > 0.0 => {
                    self.key_down(note & 0x7F, velocity, channel, timing, arp, out)
                }
                MidiEvent::NoteOn { note, .. } | MidiEvent::NoteOff { note, .. } => {
                    self.key_up(note & 0x7F, timing, arp, out)
                }
                other => emit(out, other),
            }
        }
        if let Some(arp) = arp {
            self.run_arp(arp, at as f64, block_len as f64, samples_per_tick, out);
            self.next_step_at -= block_len as f64;
            if let Some(off) = self.off_at.as_mut() {
                *off -= block_len as f64;
            }
        }
    }

    fn key_down(
        &mut self,
        key: u8,
        velocity: f32,
        channel: u8,
        timing: u32,
        arp: Option<Arp>,
        out: &mut Vec<MidiEvent>,
    ) {
        // The same key again without a release: let the first go.
        if self.keys[key as usize].count > 0 {
            self.key_up(key, timing, arp, out);
        }
        let (pitches, count) = apply(self.before_arp(), key);
        self.keys[key as usize] = KeyNotes {
            pitches,
            count: count as u8,
            velocity,
            channel,
        };
        match arp {
            None => {
                for &pitch in &pitches[..count] {
                    emit(out, note_on(timing, channel, pitch, velocity));
                }
            }
            Some(_) => {
                if self.held == 0 {
                    // The first key starts the pattern, at once.
                    self.step = 0;
                    self.next_step_at = timing as f64;
                }
                if self.held < self.order.len() {
                    self.order[self.held] = key;
                    self.held += 1;
                }
            }
        }
    }

    fn key_up(&mut self, key: u8, timing: u32, arp: Option<Arp>, out: &mut Vec<MidiEvent>) {
        let notes = self.keys[key as usize];
        self.keys[key as usize] = KeyNotes::EMPTY;
        match arp {
            None => {
                if notes.count > 0 {
                    for &pitch in notes.notes() {
                        emit(out, note_off(timing, notes.channel, pitch));
                    }
                } else {
                    // A key pressed before this node existed (a graph
                    // rebuild) or before the chain changed: what the chain
                    // makes of it now is let go, so nothing hangs. Not a
                    // note another key still holds.
                    let (pitches, count) = apply(self.before_arp(), key);
                    for &pitch in &pitches[..count] {
                        if !self.keys.iter().any(|k| k.notes().contains(&pitch)) {
                            emit(out, note_off(timing, 0, pitch));
                        }
                    }
                }
            }
            Some(_) => {
                if let Some(i) = self.order[..self.held].iter().position(|k| *k == key) {
                    self.order.copy_within(i + 1..self.held, i);
                    self.held -= 1;
                }
                if self.held == 0 {
                    self.end_step(timing, out);
                }
            }
        }
    }

    /// Let go of the arpeggio note sounding, if any.
    fn end_step(&mut self, timing: u32, out: &mut Vec<MidiEvent>) {
        let sounding = std::mem::replace(&mut self.sounding, KeyNotes::EMPTY);
        for &pitch in sounding.notes() {
            emit(out, note_off(timing, sounding.channel, pitch));
        }
        self.off_at = None;
    }

    /// The chain changed under held keys: every note they started is let
    /// go (each pitch once, so a full buffer cannot drop a release), and
    /// the keys still down play again through the new chain. An arpeggio
    /// picks the new notes up at its next step.
    fn rekey(&mut self, out: &mut Vec<MidiEvent>) {
        let mut sent = [0u128; 16];
        let mut release = |channel: u8, pitch: u8, out: &mut Vec<MidiEvent>| {
            let bit = 1u128 << (pitch & 0x7F);
            let row = &mut sent[(channel & 0x0F) as usize];
            if *row & bit == 0 {
                *row |= bit;
                emit(out, note_off(0, channel, pitch));
            }
        };
        let sounding = std::mem::replace(&mut self.sounding, KeyNotes::EMPTY);
        for &pitch in sounding.notes() {
            release(sounding.channel, pitch, out);
        }
        self.off_at = None;
        let arp = self.arp();
        for key in 0..128u8 {
            let old = self.keys[key as usize];
            if old.count == 0 {
                continue;
            }
            if arp.is_none() || self.held == 0 {
                // Notes this key started itself (with an arpeggiator they
                // only ever sounded as its steps, let go above).
                for &pitch in old.notes() {
                    release(old.channel, pitch, out);
                }
            }
            let (pitches, count) = apply(self.before_arp(), key);
            self.keys[key as usize] = KeyNotes {
                pitches,
                count: count as u8,
                ..old
            };
        }
        match arp {
            None => {
                self.held = 0;
                for key in 0..128usize {
                    let k = self.keys[key];
                    for &pitch in k.notes() {
                        emit(out, note_on(0, k.channel, pitch, k.velocity));
                    }
                }
            }
            Some(_) => {
                if self.held == 0 {
                    // Keys held before an arpeggiator came in: in the
                    // order of their numbers, starting now.
                    for key in 0..128u8 {
                        if self.keys[key as usize].count > 0 && self.held < self.order.len() {
                            self.order[self.held] = key;
                            self.held += 1;
                        }
                    }
                    self.step = 0;
                    self.next_step_at = 0.0;
                }
            }
        }
    }

    /// The arpeggiator from `from` to `to` (samples into the block): its
    /// steps and the ends of its notes that fall in between.
    fn run_arp(
        &mut self,
        arp: Arp,
        from: f64,
        to: f64,
        samples_per_tick: f64,
        out: &mut Vec<MidiEvent>,
    ) {
        let step_len = (arp.step_ticks as f64 * samples_per_tick).max(1.0);
        loop {
            let off = self.off_at.unwrap_or(f64::INFINITY);
            let step = if self.held > 0 {
                self.next_step_at.max(from)
            } else {
                f64::INFINITY
            };
            let next = off.min(step);
            if next >= to {
                return;
            }
            let timing = next.max(from) as u32;
            if off <= step {
                self.end_step(timing, out);
                continue;
            }
            // A step: the last note ends, the next one starts.
            self.end_step(timing, out);
            if let Some((pitch, velocity, channel)) = self.pick(arp) {
                let (pitches, count) = apply(self.after_arp(), pitch);
                for &p in &pitches[..count] {
                    emit(out, note_on(timing, channel, p, velocity));
                }
                self.sounding = KeyNotes {
                    pitches,
                    count: count as u8,
                    velocity,
                    channel,
                };
                self.off_at = Some(next + step_len * arp.gate as f64);
            }
            self.step += 1;
            self.next_step_at = next + step_len;
        }
    }

    /// The pitch of the current step, as the clip arpeggiator walks a
    /// chord: the held notes in the mode's order, one octave after
    /// another. With the velocity and channel of the key pressed last.
    fn pick(&self, arp: Arp) -> Option<(u8, f32, u8)> {
        let mut pitches = [0u8; ARP_PITCHES_MAX];
        let mut count = 0usize;
        for &key in &self.order[..self.held] {
            for &p in self.keys[key as usize].notes() {
                let duplicate = pitches[..count].contains(&p);
                if !duplicate && count < ARP_PITCHES_MAX {
                    pitches[count] = p;
                    count += 1;
                }
            }
        }
        if count == 0 {
            return None;
        }
        let played = pitches;
        pitches[..count].sort_unstable();
        // One octave's round, in the mode's order.
        let round_len = match arp.mode {
            ArpMode::UpDown => count + count.saturating_sub(2),
            _ => count,
        };
        let index = self.step % (round_len * arp.octaves as usize);
        let (octave, within) = (index / round_len, index % round_len);
        let base = match arp.mode {
            ArpMode::Up => pitches[within],
            ArpMode::Down => pitches[count - 1 - within],
            ArpMode::UpDown if within < count => pitches[within],
            ArpMode::UpDown => pitches[count - 2 - (within - count)],
            ArpMode::AsPlayed => played[within],
        };
        let pitch = base as i32 + octave as i32 * 12;
        let last = self.keys[self.order[self.held - 1] as usize];
        (0..=127)
            .contains(&pitch)
            .then_some((pitch as u8, last.velocity, last.channel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardwave_midi::midi_fx::ScaleKind;

    fn on(timing: u32, note: u8) -> MidiEvent {
        note_on(timing, 0, note, 0.8)
    }

    fn off(timing: u32, note: u8) -> MidiEvent {
        note_off(timing, 0, note)
    }

    /// (timing, note, on) for each note event out.
    fn notes(out: &[MidiEvent]) -> Vec<(u32, u8, bool)> {
        out.iter()
            .filter_map(|e| match *e {
                MidiEvent::NoteOn {
                    timing,
                    note,
                    velocity,
                    ..
                } if velocity > 0.0 => Some((timing, note, true)),
                MidiEvent::NoteOn { timing, note, .. }
                | MidiEvent::NoteOff { timing, note, .. } => Some((timing, note, false)),
                _ => None,
            })
            .collect()
    }

    fn run(fx: &mut LiveMidiFx, input: &[MidiEvent], block: usize) -> Vec<(u32, u8, bool)> {
        let mut out = Vec::with_capacity(256);
        fx.process(input, &mut out, block, 1.0);
        notes(&out)
    }

    #[test]
    fn a_chord_effect_plays_the_chord_and_lets_all_of_it_go() {
        let mut fx = LiveMidiFx::default();
        fx.set_chain(vec![MidiFx::Chord {
            intervals: vec![4, 7],
        }]);
        assert_eq!(
            run(&mut fx, &[on(10, 60)], 64),
            vec![(10, 60, true), (10, 64, true), (10, 67, true)]
        );
        assert_eq!(
            run(&mut fx, &[off(5, 60)], 64),
            vec![(5, 60, false), (5, 64, false), (5, 67, false)]
        );
    }

    #[test]
    fn scale_and_transpose_move_the_key_and_its_release() {
        let mut fx = LiveMidiFx::default();
        fx.set_chain(vec![
            MidiFx::Scale {
                root: 0,
                kind: ScaleKind::Major,
            },
            MidiFx::Transpose { semitones: 12 },
        ]);
        // C# is not in C major: it snaps down to C, then up an octave.
        assert_eq!(run(&mut fx, &[on(0, 61)], 64), vec![(0, 72, true)]);
        assert_eq!(run(&mut fx, &[off(0, 61)], 64), vec![(0, 72, false)]);
    }

    #[test]
    fn other_messages_pass_through_untouched() {
        let mut fx = LiveMidiFx::default();
        fx.set_chain(vec![MidiFx::Transpose { semitones: 5 }]);
        let bend = MidiEvent::PitchBend {
            timing: 3,
            channel: 0,
            value: 0.5,
        };
        let mut out = Vec::with_capacity(16);
        fx.process(&[bend], &mut out, 64, 1.0);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], MidiEvent::PitchBend { timing: 3, .. }));
    }

    #[test]
    fn the_arpeggiator_steps_through_a_held_chord_in_time() {
        let mut fx = LiveMidiFx::default();
        // A step of 100 ticks at one sample per tick: a step every 100
        // samples, each note half of it.
        fx.set_chain(vec![MidiFx::Arpeggiator {
            step_ticks: 100,
            mode: ArpMode::Up,
            gate: 0.5,
            octaves: 1,
        }]);
        let out = run(&mut fx, &[on(0, 64), on(0, 60), on(0, 67)], 256);
        assert_eq!(
            out,
            vec![
                (0, 60, true),
                (50, 60, false),
                (100, 64, true),
                (150, 64, false),
                (200, 67, true),
                (250, 67, false),
            ]
        );
        // The next block carries on where this one stopped (300 samples
        // in, the round starts again), and letting go of every key ends
        // the note sounding.
        let out = run(&mut fx, &[off(60, 60), off(60, 64), off(60, 67)], 128);
        assert_eq!(out, vec![(44, 60, true), (60, 60, false)]);
        assert!(
            run(&mut fx, &[], 512).is_empty(),
            "nothing after the keys are up"
        );
    }

    #[test]
    fn up_down_does_not_repeat_the_ends_and_octaves_climb() {
        let mut fx = LiveMidiFx::default();
        fx.set_chain(vec![MidiFx::Arpeggiator {
            step_ticks: 10,
            mode: ArpMode::UpDown,
            gate: 0.5,
            octaves: 2,
        }]);
        let out = run(&mut fx, &[on(0, 60), on(0, 64), on(0, 67)], 80);
        let starts: Vec<u8> = out.iter().filter(|n| n.2).map(|n| n.1).collect();
        assert_eq!(starts, vec![60, 64, 67, 64, 72, 76, 79, 76]);
    }

    #[test]
    fn effects_after_the_arpeggiator_work_on_each_step() {
        let mut fx = LiveMidiFx::default();
        fx.set_chain(vec![
            MidiFx::Arpeggiator {
                step_ticks: 100,
                mode: ArpMode::Up,
                gate: 0.5,
                octaves: 1,
            },
            MidiFx::Chord {
                intervals: vec![12],
            },
        ]);
        let out = run(&mut fx, &[on(0, 60)], 50);
        assert_eq!(out, vec![(0, 60, true), (0, 72, true)]);
    }

    #[test]
    fn a_rebuild_keeps_the_arpeggio_and_lets_go_of_old_notes() {
        let chain = vec![MidiFx::Arpeggiator {
            step_ticks: 100,
            mode: ArpMode::Up,
            gate: 1.0,
            octaves: 1,
        }];
        let mut old = LiveMidiFx::default();
        old.set_chain(chain.clone());
        run(&mut old, &[on(0, 60)], 50);
        // The graph is rebuilt: a new node takes over.
        let mut new = LiveMidiFx::default();
        new.set_chain(chain);
        new.take_over(old);
        let out = run(&mut new, &[off(0, 60)], 50);
        assert_eq!(
            out,
            vec![(0, 60, false)],
            "the note started before the rebuild ends"
        );
    }

    #[test]
    fn a_key_from_before_the_node_existed_is_still_let_go() {
        let mut fx = LiveMidiFx::default();
        fx.set_chain(vec![MidiFx::Transpose { semitones: 2 }]);
        assert_eq!(run(&mut fx, &[off(0, 60)], 64), vec![(0, 62, false)]);
    }

    #[test]
    fn changing_the_chain_under_a_held_key_plays_it_through_the_new_chain() {
        let mut fx = LiveMidiFx::default();
        fx.set_chain(vec![MidiFx::Transpose { semitones: 2 }]);
        run(&mut fx, &[on(0, 60)], 64);
        fx.set_chain(vec![MidiFx::Transpose { semitones: 5 }]);
        assert_eq!(run(&mut fx, &[], 64), vec![(0, 62, false), (0, 65, true)]);
        assert_eq!(run(&mut fx, &[off(0, 60)], 64), vec![(0, 65, false)]);
    }

    #[test]
    fn an_arpeggiator_added_under_held_keys_starts_on_them() {
        let mut fx = LiveMidiFx::default();
        run(&mut fx, &[on(0, 64), on(0, 60)], 64);
        fx.set_chain(vec![MidiFx::Arpeggiator {
            step_ticks: 100,
            mode: ArpMode::Up,
            gate: 0.5,
            octaves: 1,
        }]);
        let out = run(&mut fx, &[], 160);
        assert_eq!(
            out,
            vec![
                (0, 60, false),
                (0, 64, false),
                (0, 60, true),
                (50, 60, false),
                (100, 64, true),
                (150, 64, false)
            ]
        );
    }

    #[test]
    fn a_release_for_an_unknown_key_spares_a_note_another_key_holds() {
        let mut fx = LiveMidiFx::default();
        fx.set_chain(vec![MidiFx::Scale {
            root: 0,
            kind: ScaleKind::Major,
        }]);
        // 60 sounds as 60; a release of 61 (pressed before this node
        // existed) maps to 60 too, and must not cut the key still down.
        run(&mut fx, &[on(0, 60)], 64);
        assert!(run(&mut fx, &[off(0, 61)], 64).is_empty());
    }

    #[test]
    fn a_full_buffer_drops_notes_rather_than_grow() {
        let mut fx = LiveMidiFx::default();
        fx.set_chain(vec![MidiFx::Chord {
            intervals: vec![1, 2, 3, 4, 5],
        }]);
        let mut out = Vec::with_capacity(4);
        fx.process(&[on(0, 60)], &mut out, 64, 1.0);
        assert_eq!(out.len(), 4);
        assert_eq!(out.capacity(), 4);
    }
}
