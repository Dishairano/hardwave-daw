//! Tuning a sung line, note by note.
//!
//! The readiness list called this "pitch correction per note" and
//! noted that the phase vocoder it needs exists while the editing does
//! not. This is the editing: find each sung note, see how far it sits
//! from where it should be, and move it there.
//!
//! Each note is shifted by its own amount rather than the line by one
//! amount, which is the whole point: a singer is sharp on one word and
//! flat on the next.

use crate::audio_to_midi::{self, DetectedNote, Settings};

/// What one note needed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Correction {
    pub start_sample: u64,
    pub length_samples: u64,
    /// The note it was nearest.
    pub pitch: u8,
    /// Where it sat, in cents from that note. Negative is flat.
    pub was_off_cents: f32,
    /// What was taken out, in cents. With strength below one, this is
    /// less than it was off by.
    pub moved_cents: f32,
}

/// Notes of the scale, as semitones from the root, for snapping to a
/// key rather than to the nearest black or white note.
pub const MAJOR: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
pub const MINOR: [u8; 7] = [0, 2, 3, 5, 7, 8, 10];
pub const CHROMATIC: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

/// How to tune.
#[derive(Debug, Clone)]
pub struct Tuning {
    /// 0 leaves it alone, 1 puts every note exactly in tune. Hard
    /// dance wants 1; a sung chorus usually does not.
    pub strength: f32,
    /// The root of the key, as a MIDI pitch class, 0 for C.
    pub root: u8,
    /// Which degrees of the scale count as in tune.
    pub scale: Vec<u8>,
    /// Leave a note alone when it was already this close, so vibrato
    /// and a human edge survive.
    pub ignore_within_cents: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            strength: 1.0,
            root: 0,
            scale: CHROMATIC.to_vec(),
            ignore_within_cents: 0.0,
        }
    }
}

/// The nearest note of the key to a given pitch.
pub fn snap_to_scale(pitch: u8, root: u8, scale: &[u8]) -> u8 {
    if scale.is_empty() {
        return pitch;
    }
    let mut best = pitch;
    let mut best_distance = i32::MAX;
    // Look an octave either way: the nearest note of the key can be
    // over the octave line.
    for octave in -1i32..=1 {
        for degree in scale {
            let candidate =
                (pitch as i32 / 12 + octave) * 12 + ((root as i32 + *degree as i32) % 12);
            if !(0..=127).contains(&candidate) {
                continue;
            }
            let distance = (candidate - pitch as i32).abs();
            if distance < best_distance {
                best_distance = distance;
                best = candidate as u8;
            }
        }
    }
    best
}

/// Tune a recording and say what was done to it.
///
/// The audio comes back the same length: each note is shifted in
/// place, so nothing moves in time and the take still lines up with
/// the rest of the song.
pub fn tune(samples: &[f32], settings: &Settings, tuning: &Tuning) -> (Vec<f32>, Vec<Correction>) {
    let heard = audio_to_midi::transcribe(samples, settings);
    tune_notes(samples, &heard, tuning)
}

/// The same, with the notes already found. Separate so an editor can
/// show them, let them be changed by hand, and then apply.
pub fn tune_notes(
    samples: &[f32],
    notes: &[DetectedNote],
    tuning: &Tuning,
) -> (Vec<f32>, Vec<Correction>) {
    let mut out = samples.to_vec();
    let mut corrections = Vec::with_capacity(notes.len());

    for note in notes {
        let start = note.start_sample as usize;
        let end = (start + note.length_samples as usize).min(samples.len());
        if start >= end {
            continue;
        }
        let target = snap_to_scale(note.pitch, tuning.root, &tuning.scale);
        // How far the sung pitch is from where it should end up:
        // what it was off by, plus the move to another note of the key.
        let distance_cents = note.cents_off + (target as f32 - note.pitch as f32) * 100.0;
        if distance_cents.abs() <= tuning.ignore_within_cents {
            corrections.push(Correction {
                start_sample: note.start_sample,
                length_samples: note.length_samples,
                pitch: note.pitch,
                was_off_cents: note.cents_off,
                moved_cents: 0.0,
            });
            continue;
        }
        let moved = -distance_cents * tuning.strength.clamp(0.0, 1.0);
        if moved.abs() < 1.0 {
            // Under a cent is below what anyone hears, and shifting by
            // it would cost the quality of the vocoder for nothing.
            corrections.push(Correction {
                start_sample: note.start_sample,
                length_samples: note.length_samples,
                pitch: note.pitch,
                was_off_cents: note.cents_off,
                moved_cents: 0.0,
            });
            continue;
        }

        let shifted = crate::phase_vocoder::pitch_shift_pv(&samples[start..end], moved / 100.0);
        // The vocoder can give back a length of its own; the take has
        // to stay where it is, so it is fitted back into the gap.
        let fitted = if shifted.len() == end - start {
            shifted
        } else {
            crate::time_pitch::resample_to_length(&shifted, end - start)
        };
        out[start..end].copy_from_slice(&fitted);

        corrections.push(Correction {
            start_sample: note.start_sample,
            length_samples: note.length_samples,
            pitch: note.pitch,
            was_off_cents: note.cents_off,
            moved_cents: moved,
        });
    }

    (out, corrections)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, seconds: f64, sample_rate: f64) -> Vec<f32> {
        let n = (seconds * sample_rate) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / sample_rate;
                let f = std::f64::consts::TAU * hz * t;
                (f.sin() * 0.6 + (2.0 * f).sin() * 0.25 + (3.0 * f).sin() * 0.12) as f32
            })
            .collect()
    }

    #[test]
    fn the_nearest_note_of_the_key_is_found() {
        // In C major, C# is not in the key: C is nearer than D.
        assert_eq!(snap_to_scale(61, 0, &MAJOR), 60);
        // D is in the key and stays put.
        assert_eq!(snap_to_scale(62, 0, &MAJOR), 62);
        // In A minor, G# is out: A is nearer than G.
        assert_eq!(snap_to_scale(68, 9, &MINOR), 69);
        // Chromatic leaves everything alone.
        for pitch in 48..72 {
            assert_eq!(snap_to_scale(pitch, 0, &CHROMATIC), pitch);
        }
    }

    #[test]
    fn a_flat_note_is_reported_as_flat_and_moved_up() {
        let settings = Settings::default();
        // 30 cents flat of A3 (220 Hz).
        let flat = 220.0 * 2f64.powf(-30.0 / 1200.0);
        let audio = tone(flat, 0.6, settings.sample_rate);
        let (out, corrections) = tune(&audio, &settings, &Tuning::default());
        assert_eq!(out.len(), audio.len(), "the take must not change length");
        assert_eq!(corrections.len(), 1, "{corrections:?}");
        let correction = corrections[0];
        assert!(
            correction.was_off_cents < -15.0,
            "it should read as flat: {correction:?}"
        );
        assert!(
            correction.moved_cents > 10.0,
            "it should be moved up: {correction:?}"
        );
    }

    #[test]
    fn a_note_already_in_tune_is_left_alone() {
        let settings = Settings::default();
        let audio = tone(220.0, 0.6, settings.sample_rate);
        let (out, corrections) = tune(&audio, &settings, &Tuning::default());
        assert_eq!(corrections.len(), 1);
        assert!(
            corrections[0].moved_cents.abs() < 12.0,
            "a note in tune should barely move: {:?}",
            corrections[0]
        );
        assert_eq!(out.len(), audio.len());
    }

    #[test]
    fn strength_scales_how_far_a_note_moves() {
        let settings = Settings::default();
        let flat = 220.0 * 2f64.powf(-40.0 / 1200.0);
        let audio = tone(flat, 0.6, settings.sample_rate);
        let full = tune(&audio, &settings, &Tuning::default()).1[0].moved_cents;
        let half = tune(
            &audio,
            &settings,
            &Tuning {
                strength: 0.5,
                ..Default::default()
            },
        )
        .1[0]
            .moved_cents;
        assert!(
            (half * 2.0 - full).abs() < 2.0,
            "half strength should move half as far: {half} against {full}"
        );
    }

    #[test]
    fn a_note_inside_the_tolerance_is_not_touched() {
        let settings = Settings::default();
        let flat = 220.0 * 2f64.powf(-20.0 / 1200.0);
        let audio = tone(flat, 0.6, settings.sample_rate);
        let (out, corrections) = tune(
            &audio,
            &settings,
            &Tuning {
                ignore_within_cents: 50.0,
                ..Default::default()
            },
        );
        assert_eq!(corrections[0].moved_cents, 0.0);
        assert_eq!(out, audio, "nothing should have been rewritten");
    }

    #[test]
    fn silence_has_nothing_to_tune() {
        let settings = Settings::default();
        let audio = vec![0.0f32; 48_000];
        let (out, corrections) = tune(&audio, &settings, &Tuning::default());
        assert!(corrections.is_empty());
        assert_eq!(out, audio);
    }
}
