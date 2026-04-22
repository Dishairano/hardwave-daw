//! MIDI recording state machine + note accumulator. Mirrors the
//! audio RecordingSession but for note-on / note-off events. Handles
//! overdub (add notes on top of existing) and replace (clear then
//! record) modes, plus optional input quantization.

use crate::MidiNote;

/// What to do with existing notes when recording starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiRecordMode {
    /// Overdub — new notes are added to the existing list, keeping
    /// any previous recording intact.
    Overdub,
    /// Replace — existing notes in the recorded tick range are
    /// cleared before capture.
    Replace,
}

/// One in-flight note-on event waiting for its matching note-off.
#[derive(Clone)]
struct PendingNote {
    start_tick: u64,
    pitch: u8,
    velocity: f32,
    channel: u8,
}

/// MIDI recording session. Accumulates note-on / note-off pairs into
/// `MidiNote`s plus optional input quantization to a grid.
pub struct MidiRecorder {
    mode: MidiRecordMode,
    notes: Vec<MidiNote>,
    pending: Vec<PendingNote>,
    quantize_ticks: Option<u64>,
    recording: bool,
    /// Optional range to clear on Replace mode. None = clear everything.
    replace_range_ticks: Option<(u64, u64)>,
}

impl Default for MidiRecorder {
    fn default() -> Self {
        Self {
            mode: MidiRecordMode::Overdub,
            notes: Vec::new(),
            pending: Vec::new(),
            quantize_ticks: None,
            recording: false,
            replace_range_ticks: None,
        }
    }
}

impl MidiRecorder {
    pub fn set_mode(&mut self, mode: MidiRecordMode) {
        self.mode = mode;
    }

    pub fn mode(&self) -> MidiRecordMode {
        self.mode
    }

    /// Set input quantization grid. `None` = no quantize (notes land
    /// exactly where they're played); `Some(ticks)` snaps note start
    /// ticks to the nearest multiple of `ticks`.
    pub fn set_quantize(&mut self, ticks: Option<u64>) {
        self.quantize_ticks = ticks;
    }

    pub fn set_replace_range(&mut self, range: Option<(u64, u64)>) {
        self.replace_range_ticks = range;
    }

    /// Seed the recorder with existing notes (e.g. for overdub mode
    /// where the current pattern's notes are preserved underneath).
    pub fn seed_notes(&mut self, notes: Vec<MidiNote>) {
        self.notes = notes;
    }

    pub fn start(&mut self) {
        self.recording = true;
        self.pending.clear();
        if self.mode == MidiRecordMode::Replace {
            if let Some((start, end)) = self.replace_range_ticks {
                self.notes
                    .retain(|n| n.start_tick < start || n.start_tick >= end);
            } else {
                self.notes.clear();
            }
        }
    }

    pub fn stop(&mut self) {
        self.recording = false;
        // Close any pending notes at the current cursor position.
        let end_tick = self.current_latest_tick();
        self.pending.retain(|p| {
            self.notes.push(MidiNote {
                start_tick: p.start_tick,
                duration_ticks: end_tick.saturating_sub(p.start_tick).max(1),
                pitch: p.pitch,
                velocity: p.velocity,
                channel: p.channel,
                muted: false,
            });
            false
        });
    }

    pub fn is_recording(&self) -> bool {
        self.recording
    }

    pub fn note_count(&self) -> usize {
        self.notes.len()
    }

    pub fn notes(&self) -> &[MidiNote] {
        &self.notes
    }

    pub fn take_notes(&mut self) -> Vec<MidiNote> {
        std::mem::take(&mut self.notes)
    }

    /// Record a note-on event. No-op if not recording.
    pub fn note_on(&mut self, tick: u64, pitch: u8, velocity: f32, channel: u8) {
        if !self.recording {
            return;
        }
        let quantized = self.quantize(tick);
        self.pending.push(PendingNote {
            start_tick: quantized,
            pitch,
            velocity: velocity.clamp(0.0, 1.0),
            channel,
        });
    }

    /// Record a note-off event. Closes the matching pending note into
    /// a full `MidiNote` with a duration. No-op if not recording or
    /// no matching note-on is pending.
    pub fn note_off(&mut self, tick: u64, pitch: u8, channel: u8) {
        if !self.recording {
            return;
        }
        if let Some(pos) = self
            .pending
            .iter()
            .position(|p| p.pitch == pitch && p.channel == channel)
        {
            let pending = self.pending.remove(pos);
            let end_tick = self.quantize(tick);
            self.notes.push(MidiNote {
                start_tick: pending.start_tick,
                duration_ticks: end_tick.saturating_sub(pending.start_tick).max(1),
                pitch: pending.pitch,
                velocity: pending.velocity,
                channel: pending.channel,
                muted: false,
            });
        }
    }

    fn quantize(&self, tick: u64) -> u64 {
        match self.quantize_ticks {
            Some(q) if q > 0 => ((tick + q / 2) / q) * q,
            _ => tick,
        }
    }

    fn current_latest_tick(&self) -> u64 {
        self.notes
            .iter()
            .map(|n| n.start_tick + n.duration_ticks)
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overdub_preserves_seeded_notes() {
        let mut rec = MidiRecorder::default();
        rec.set_mode(MidiRecordMode::Overdub);
        rec.seed_notes(vec![MidiNote {
            start_tick: 0,
            duration_ticks: 480,
            pitch: 60,
            velocity: 0.8,
            channel: 0,
            muted: false,
        }]);
        rec.start();
        rec.note_on(960, 64, 0.7, 0);
        rec.note_off(1200, 64, 0);
        rec.stop();
        // Should have both the seeded note and the newly-recorded one.
        assert_eq!(rec.note_count(), 2);
    }

    #[test]
    fn replace_mode_clears_notes_before_capture() {
        let mut rec = MidiRecorder::default();
        rec.set_mode(MidiRecordMode::Replace);
        rec.seed_notes(vec![
            MidiNote {
                start_tick: 0,
                duration_ticks: 480,
                pitch: 60,
                velocity: 0.8,
                channel: 0,
                muted: false,
            },
            MidiNote {
                start_tick: 960,
                duration_ticks: 480,
                pitch: 64,
                velocity: 0.8,
                channel: 0,
                muted: false,
            },
        ]);
        rec.start();
        rec.note_on(500, 67, 0.9, 0);
        rec.note_off(700, 67, 0);
        rec.stop();
        // Seeded notes should all be cleared.
        assert_eq!(rec.note_count(), 1);
        assert_eq!(rec.notes()[0].pitch, 67);
    }

    #[test]
    fn replace_range_only_clears_specified_ticks() {
        let mut rec = MidiRecorder::default();
        rec.set_mode(MidiRecordMode::Replace);
        rec.seed_notes(vec![
            MidiNote {
                start_tick: 0,
                duration_ticks: 100,
                pitch: 60,
                velocity: 0.8,
                channel: 0,
                muted: false,
            },
            MidiNote {
                start_tick: 1000,
                duration_ticks: 100,
                pitch: 64,
                velocity: 0.8,
                channel: 0,
                muted: false,
            },
            MidiNote {
                start_tick: 2000,
                duration_ticks: 100,
                pitch: 67,
                velocity: 0.8,
                channel: 0,
                muted: false,
            },
        ]);
        // Replace only ticks [500, 1500).
        rec.set_replace_range(Some((500, 1500)));
        rec.start();
        rec.stop();
        // Notes at 0 and 2000 survive; note at 1000 was cleared.
        assert_eq!(rec.note_count(), 2);
        let pitches: Vec<u8> = rec.notes().iter().map(|n| n.pitch).collect();
        assert!(pitches.contains(&60));
        assert!(pitches.contains(&67));
        assert!(!pitches.contains(&64));
    }

    #[test]
    fn note_on_off_produces_one_midi_note() {
        let mut rec = MidiRecorder::default();
        rec.start();
        rec.note_on(0, 60, 0.75, 0);
        rec.note_off(480, 60, 0);
        rec.stop();
        assert_eq!(rec.note_count(), 1);
        let n = &rec.notes()[0];
        assert_eq!(n.start_tick, 0);
        assert_eq!(n.duration_ticks, 480);
        assert_eq!(n.pitch, 60);
        assert_eq!(n.velocity, 0.75);
    }

    #[test]
    fn velocity_clamps_to_normalized_range() {
        let mut rec = MidiRecorder::default();
        rec.start();
        rec.note_on(0, 60, 1.5, 0);
        rec.note_off(100, 60, 0);
        assert_eq!(rec.notes()[0].velocity, 1.0);
        rec.note_on(200, 62, -0.5, 0);
        rec.note_off(300, 62, 0);
        assert_eq!(rec.notes()[1].velocity, 0.0);
    }

    #[test]
    fn quantize_snaps_note_starts_to_grid() {
        let mut rec = MidiRecorder::default();
        rec.set_quantize(Some(120));
        rec.start();
        // 100 rounds down to 120 (nearest multiple), 250 to 240.
        rec.note_on(100, 60, 0.7, 0);
        rec.note_off(200, 60, 0);
        rec.note_on(250, 64, 0.7, 0);
        rec.note_off(350, 64, 0);
        rec.stop();
        assert_eq!(rec.notes()[0].start_tick, 120);
        assert_eq!(rec.notes()[1].start_tick, 240);
    }

    #[test]
    fn stop_without_matching_note_off_closes_pending() {
        let mut rec = MidiRecorder::default();
        rec.start();
        rec.note_on(0, 60, 0.7, 0);
        // No note_off — stopping should still produce a note.
        rec.stop();
        assert_eq!(rec.note_count(), 1);
    }

    #[test]
    fn events_outside_recording_are_ignored() {
        let mut rec = MidiRecorder::default();
        rec.note_on(0, 60, 0.7, 0);
        rec.note_off(100, 60, 0);
        assert_eq!(rec.note_count(), 0);
    }

    #[test]
    fn take_notes_consumes_the_buffer() {
        let mut rec = MidiRecorder::default();
        rec.start();
        rec.note_on(0, 60, 0.7, 0);
        rec.note_off(100, 60, 0);
        rec.stop();
        let notes = rec.take_notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(rec.note_count(), 0);
    }
}
