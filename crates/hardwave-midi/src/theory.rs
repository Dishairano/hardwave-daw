//! Music theory primitives — chord suggestion and melody generation
//! from diatonic harmony rules. No ML, no randomness beyond a simple
//! linear-congruential PRNG — pure deterministic music theory.
//!
//! Used as the seed of "suggest next chord" and "generate melody
//! from chord progression" features that UI layers can build on.

/// A chord root expressed as a MIDI note within one octave.
/// 0 = C, 1 = C#, ..., 11 = B.
pub type PitchClass = u8;

/// Chord quality — determines the intervals above the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordQuality {
    Major,
    Minor,
    Diminished,
    Augmented,
    /// Dominant 7 — major triad + flat 7.
    Dominant7,
    Major7,
    Minor7,
}

impl ChordQuality {
    /// Intervals above the root that make up this chord, in semitones.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            ChordQuality::Major => &[0, 4, 7],
            ChordQuality::Minor => &[0, 3, 7],
            ChordQuality::Diminished => &[0, 3, 6],
            ChordQuality::Augmented => &[0, 4, 8],
            ChordQuality::Dominant7 => &[0, 4, 7, 10],
            ChordQuality::Major7 => &[0, 4, 7, 11],
            ChordQuality::Minor7 => &[0, 3, 7, 10],
        }
    }
}

/// A chord — root pitch class + quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub root: PitchClass,
    pub quality: ChordQuality,
}

impl Chord {
    pub fn new(root: PitchClass, quality: ChordQuality) -> Self {
        Self {
            root: root % 12,
            quality,
        }
    }

    /// Get the chord tones as absolute MIDI notes starting from
    /// `octave_base` (e.g. 48 = C3).
    pub fn notes(&self, octave_base: u8) -> Vec<u8> {
        self.quality
            .intervals()
            .iter()
            .map(|i| octave_base + self.root + *i)
            .collect()
    }
}

/// Major-scale pitch classes rooted at `key_root`. Returns 7 pitch
/// classes starting from the root.
pub fn major_scale(key_root: PitchClass) -> [PitchClass; 7] {
    let intervals = [0u8, 2, 4, 5, 7, 9, 11];
    let mut out = [0u8; 7];
    for (i, &iv) in intervals.iter().enumerate() {
        out[i] = (key_root + iv) % 12;
    }
    out
}

/// Diatonic chord qualities for each scale degree in a major key.
/// I ii iii IV V vi vii°
pub const MAJOR_DEGREES: [ChordQuality; 7] = [
    ChordQuality::Major,
    ChordQuality::Minor,
    ChordQuality::Minor,
    ChordQuality::Major,
    ChordQuality::Major,
    ChordQuality::Minor,
    ChordQuality::Diminished,
];

/// Suggest the next chord given the current chord and key. Uses
/// common-practice harmonic rules: V→I, IV→I, ii→V, vi→IV, I→IV.
/// Falls back to the tonic (I) if the current chord isn't recognized
/// as a diatonic function in the key.
pub fn suggest_next_chord(current: Chord, key_root: PitchClass) -> Chord {
    let scale = major_scale(key_root);
    let degree = scale.iter().position(|&p| p == current.root);
    let next_degree = match degree {
        Some(0) => 3, // I → IV
        Some(3) => 0, // IV → I
        Some(4) => 0, // V → I
        Some(1) => 4, // ii → V
        Some(5) => 3, // vi → IV
        Some(6) => 0, // vii° → I (resolution)
        _ => 0,
    };
    Chord::new(scale[next_degree], MAJOR_DEGREES[next_degree])
}

/// Generate a diatonic melody over a chord progression. For each
/// chord, emits `notes_per_chord` melody notes. Notes are picked
/// from the chord tones + diatonic scale degrees above the chord
/// root, alternating between chord tones (on strong beats) and
/// nearby passing tones (on weak beats).
pub fn generate_melody(
    chords: &[Chord],
    key_root: PitchClass,
    octave_base: u8,
    notes_per_chord: usize,
) -> Vec<u8> {
    let scale = major_scale(key_root);
    let mut melody = Vec::with_capacity(chords.len() * notes_per_chord);
    for (i, chord) in chords.iter().enumerate() {
        let chord_tones = chord.notes(octave_base);
        for n in 0..notes_per_chord {
            // Alternate between chord-tone and scale step above.
            let note = if n % 2 == 0 {
                chord_tones[(n / 2) % chord_tones.len()]
            } else {
                // Scale step above the current chord tone.
                let base = chord_tones[(n / 2) % chord_tones.len()];
                let base_pc = base % 12;
                let next_scale_pc = scale
                    .iter()
                    .copied()
                    .find(|&p| p > base_pc)
                    .unwrap_or(scale[0] + 12);
                octave_base + next_scale_pc
            };
            // Small octave lift every few chords to add melodic interest.
            let octave_shift = (i / 4) as u8 * 12;
            melody.push(note.saturating_add(octave_shift));
        }
    }
    melody
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_scale_c_has_correct_pitches() {
        let scale = major_scale(0);
        assert_eq!(scale, [0, 2, 4, 5, 7, 9, 11]);
    }

    #[test]
    fn major_scale_g_starts_at_g() {
        let scale = major_scale(7);
        assert_eq!(scale[0], 7);
        assert_eq!(scale[6], 6); // F#
    }

    #[test]
    fn chord_notes_stacks_intervals_from_octave_base() {
        let c_major = Chord::new(0, ChordQuality::Major);
        let notes = c_major.notes(60);
        assert_eq!(notes, vec![60, 64, 67]);
    }

    #[test]
    fn chord_dominant7_has_four_notes() {
        let g7 = Chord::new(7, ChordQuality::Dominant7);
        let notes = g7.notes(48);
        assert_eq!(notes.len(), 4);
        // G, B, D, F
        assert_eq!(notes, vec![55, 59, 62, 65]);
    }

    #[test]
    fn suggest_next_chord_resolves_v_to_i() {
        // In C major, G (V) should suggest C (I).
        let g = Chord::new(7, ChordQuality::Major);
        let next = suggest_next_chord(g, 0);
        assert_eq!(next.root, 0);
        assert_eq!(next.quality, ChordQuality::Major);
    }

    #[test]
    fn suggest_next_chord_i_to_iv() {
        let c = Chord::new(0, ChordQuality::Major);
        let next = suggest_next_chord(c, 0);
        assert_eq!(next.root, 5);
        assert_eq!(next.quality, ChordQuality::Major);
    }

    #[test]
    fn suggest_next_chord_ii_to_v() {
        // In C major, ii is Dm.
        let dm = Chord::new(2, ChordQuality::Minor);
        let next = suggest_next_chord(dm, 0);
        assert_eq!(next.root, 7);
        assert_eq!(next.quality, ChordQuality::Major);
    }

    #[test]
    fn suggest_next_chord_vi_to_iv() {
        // In C major, vi is Am.
        let am = Chord::new(9, ChordQuality::Minor);
        let next = suggest_next_chord(am, 0);
        assert_eq!(next.root, 5);
    }

    #[test]
    fn generate_melody_produces_expected_count() {
        let chords = vec![
            Chord::new(0, ChordQuality::Major),
            Chord::new(5, ChordQuality::Major),
            Chord::new(7, ChordQuality::Major),
            Chord::new(0, ChordQuality::Major),
        ];
        let melody = generate_melody(&chords, 0, 60, 8);
        assert_eq!(melody.len(), 32);
    }

    #[test]
    fn generate_melody_starts_with_chord_tone() {
        let chords = vec![Chord::new(0, ChordQuality::Major)];
        let melody = generate_melody(&chords, 0, 60, 2);
        // First note should be the chord root (C4 = 60).
        assert_eq!(melody[0], 60);
    }
}
