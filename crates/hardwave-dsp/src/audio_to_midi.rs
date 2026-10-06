//! Turning a sung or played line into notes.
//!
//! A hummed melody is the fastest way to get an idea down, and
//! retyping it in the piano roll is the slowest. This listens to a
//! monophonic recording, follows the pitch, and writes the notes it
//! heard.
//!
//! One voice at a time. A chord is several pitches at once and picking
//! them apart is a different problem; here a chord comes out as
//! whichever note is loudest, which is honest rather than wrong.

/// A note heard in a recording.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectedNote {
    /// Where it starts, in samples from the beginning of the audio.
    pub start_sample: u64,
    pub length_samples: u64,
    /// MIDI note number.
    pub pitch: u8,
    /// How loud it was, 0 to 1.
    pub velocity: f32,
    /// How far the sung pitch sat from the note, in cents. Useful for
    /// saying "this was flat" rather than silently rounding.
    pub cents_off: f32,
}

/// What to listen for.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub sample_rate: f64,
    /// The lowest note to look for. Below this, a guess is usually the
    /// octave error that pitch tracking is famous for.
    pub lowest_hz: f64,
    pub highest_hz: f64,
    /// How loud a window has to be to count as sung rather than as the
    /// room, as a linear amplitude.
    pub silence_floor: f32,
    /// How many windows in a row must agree before a new note starts.
    /// One window is a wobble; three is a note.
    pub stability_windows: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            lowest_hz: 65.0,    // C2, below most singing
            highest_hz: 1200.0, // D6
            silence_floor: 0.01,
            stability_windows: 3,
        }
    }
}

/// The pitch of one window, in Hz, or nothing when there is no pitch
/// in it.
///
/// Autocorrelation with the first strong peak taken rather than the
/// highest: the highest is as often an octave down, which is what
/// makes a transcription sound wrong in a way that is hard to find.
pub fn window_pitch(window: &[f32], settings: &Settings) -> Option<f64> {
    let n = window.len();
    if n < 128 {
        return None;
    }
    let energy: f32 = window.iter().map(|s| s * s).sum::<f32>() / n as f32;
    if energy.sqrt() < settings.silence_floor {
        return None;
    }

    let min_lag = (settings.sample_rate / settings.highest_hz)
        .floor()
        .max(2.0) as usize;
    let max_lag = (settings.sample_rate / settings.lowest_hz).ceil() as usize;
    let max_lag = max_lag.min(n / 2);
    if min_lag >= max_lag {
        return None;
    }

    // Normalised difference, the heart of YIN: small where the wave
    // repeats itself.
    let mut difference = vec![0.0f64; max_lag + 1];
    for (lag, slot) in difference
        .iter_mut()
        .enumerate()
        .take(max_lag + 1)
        .skip(min_lag)
    {
        let mut sum = 0.0f64;
        for i in 0..(n - lag) {
            let d = window[i] as f64 - window[i + lag] as f64;
            sum += d * d;
        }
        *slot = sum / (n - lag) as f64;
    }

    // Cumulative mean normalisation, so the measure is comparable
    // across lags and a threshold means the same thing everywhere.
    let mut running = 0.0f64;
    let mut normalised = vec![1.0f64; max_lag + 1];
    for lag in min_lag..=max_lag {
        running += difference[lag];
        if running > 0.0 {
            normalised[lag] = difference[lag] * (lag - min_lag + 1) as f64 / running;
        }
    }

    // The first lag that dips under the threshold wins, not the
    // deepest: the deepest is often twice the period, an octave down.
    const THRESHOLD: f64 = 0.15;
    let mut best_lag = None;
    for lag in min_lag..=max_lag {
        if normalised[lag] < THRESHOLD {
            let mut local = lag;
            while local < max_lag && normalised[local + 1] < normalised[local] {
                local += 1;
            }
            best_lag = Some(local);
            break;
        }
    }
    let lag = best_lag.or_else(|| {
        // Nothing clear: take the best there is, but only if it is at
        // least somewhat periodic.
        let (lag, value) = (min_lag..=max_lag)
            .map(|l| (l, normalised[l]))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
        (value < 0.4).then_some(lag)
    })?;

    // Parabolic interpolation around the dip, so the pitch is not
    // quantised to whole samples. At 1 kHz a whole sample is a third
    // of a semitone, which would read as out of tune.
    let refined = if lag > min_lag && lag < max_lag {
        let (a, b, c) = (normalised[lag - 1], normalised[lag], normalised[lag + 1]);
        let denominator = 2.0 * (2.0 * b - a - c);
        if denominator.abs() > f64::EPSILON {
            lag as f64 + (c - a) / denominator
        } else {
            lag as f64
        }
    } else {
        lag as f64
    };

    Some(settings.sample_rate / refined)
}

/// The MIDI note a frequency is nearest, and how far off it sat.
pub fn hz_to_note(hz: f64) -> (u8, f32) {
    let exact = 69.0 + 12.0 * (hz / 440.0).log2();
    let nearest = exact.round();
    let cents = ((exact - nearest) * 100.0) as f32;
    (nearest.clamp(0.0, 127.0) as u8, cents)
}

/// Listen to a recording and write down the notes.
pub fn transcribe(samples: &[f32], settings: &Settings) -> Vec<DetectedNote> {
    // A window long enough to hold two cycles of the lowest note we
    // look for, stepped by a quarter of itself.
    let window_len = ((settings.sample_rate / settings.lowest_hz) * 2.0).ceil() as usize;
    let window_len = window_len.max(1024);
    let hop = (window_len / 4).max(128);
    if samples.len() < window_len {
        return Vec::new();
    }

    let mut notes: Vec<DetectedNote> = Vec::new();
    let mut current: Option<(u8, u64, usize, f32)> = None; // pitch, start, windows, peak
    let mut pending: Option<(u8, usize)> = None; // a pitch waiting to be believed

    let mut position = 0usize;
    while position + window_len <= samples.len() {
        let window = &samples[position..position + window_len];
        let peak = window.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let heard = window_pitch(window, settings).map(hz_to_note);

        match heard {
            Some((pitch, _cents)) => {
                let same_as_current = current.map(|c| c.0) == Some(pitch);
                if same_as_current {
                    if let Some(c) = current.as_mut() {
                        c.2 += 1;
                        c.3 = c.3.max(peak);
                    }
                    pending = None;
                } else {
                    // A different pitch has to hold for a few windows
                    // before it ends the note: one window of wobble at
                    // the start of a sung note is not a new note.
                    pending = match pending {
                        Some((p, count)) if p == pitch => Some((p, count + 1)),
                        _ => Some((pitch, 1)),
                    };
                    if pending.map(|p| p.1).unwrap_or(0) >= settings.stability_windows {
                        if let Some((p, start, windows, loudest)) = current.take() {
                            push_note(&mut notes, p, start, windows, hop, window_len, loudest);
                        }
                        current = Some((pitch, position as u64, 1, peak));
                        pending = None;
                    }
                }
            }
            None => {
                // Silence ends a note straight away: a rest is a rest.
                if let Some((p, start, windows, loudest)) = current.take() {
                    push_note(&mut notes, p, start, windows, hop, window_len, loudest);
                }
                pending = None;
            }
        }
        position += hop;
    }
    if let Some((p, start, windows, loudest)) = current.take() {
        push_note(&mut notes, p, start, windows, hop, window_len, loudest);
    }
    notes
}

#[allow(clippy::too_many_arguments)]
fn push_note(
    notes: &mut Vec<DetectedNote>,
    pitch: u8,
    start_sample: u64,
    windows: usize,
    hop: usize,
    window_len: usize,
    peak: f32,
) {
    let length = (windows.saturating_sub(1) * hop + window_len) as u64;
    // A note shorter than a thirty-second at 120 bpm is a glitch in the
    // tracking rather than something anyone sang.
    if length < (window_len as u64) {
        return;
    }
    notes.push(DetectedNote {
        start_sample,
        length_samples: length,
        pitch,
        velocity: peak.clamp(0.0, 1.0),
        cents_off: 0.0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, seconds: f64, sample_rate: f64) -> Vec<f32> {
        let n = (seconds * sample_rate) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / sample_rate;
                // A few harmonics, because a voice is not a sine and a
                // tracker that only survives sines is no use.
                let f = std::f64::consts::TAU * hz * t;
                (f.sin() * 0.6 + (2.0 * f).sin() * 0.25 + (3.0 * f).sin() * 0.12) as f32
            })
            .collect()
    }

    #[test]
    fn a_held_note_is_heard_as_that_note() {
        let settings = Settings::default();
        let audio = tone(220.0, 0.5, settings.sample_rate);
        let notes = transcribe(&audio, &settings);
        assert_eq!(notes.len(), 1, "one held note, not {notes:?}");
        assert_eq!(notes[0].pitch, 57, "A3 is MIDI 57");
    }

    #[test]
    fn a_window_follows_the_pitch_it_is_given() {
        let settings = Settings::default();
        for hz in [82.41, 110.0, 220.0, 440.0, 880.0] {
            let audio = tone(hz, 0.2, settings.sample_rate);
            let heard = window_pitch(&audio[..4096], &settings).expect("a pitch");
            let cents = 1200.0 * (heard / hz).log2();
            assert!(
                cents.abs() < 25.0,
                "{hz} Hz came back as {heard} Hz, {cents:.0} cents out"
            );
        }
    }

    #[test]
    fn two_notes_in_a_row_come_back_as_two() {
        let settings = Settings::default();
        let mut audio = tone(220.0, 0.4, settings.sample_rate);
        audio.extend(tone(330.0, 0.4, settings.sample_rate));
        let notes = transcribe(&audio, &settings);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert_eq!(notes[0].pitch, 57);
        assert_eq!(notes[1].pitch, 64, "E4 is MIDI 64");
        assert!(notes[1].start_sample > notes[0].start_sample);
    }

    #[test]
    fn a_rest_ends_a_note() {
        let settings = Settings::default();
        let mut audio = tone(220.0, 0.3, settings.sample_rate);
        audio.extend(vec![0.0f32; (0.3 * settings.sample_rate) as usize]);
        audio.extend(tone(220.0, 0.3, settings.sample_rate));
        let notes = transcribe(&audio, &settings);
        assert_eq!(notes.len(), 2, "the same note twice, not held: {notes:?}");
    }

    #[test]
    fn silence_is_not_a_melody() {
        let settings = Settings::default();
        let audio = vec![0.0f32; 48_000];
        assert!(transcribe(&audio, &settings).is_empty());
    }

    #[test]
    fn noise_does_not_become_notes() {
        let settings = Settings::default();
        let mut seed = 12345u32;
        let audio: Vec<f32> = (0..48_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                ((seed >> 16) % 1000) as f32 / 500.0 - 1.0
            })
            .collect();
        let notes = transcribe(&audio, &settings);
        assert!(
            notes.len() < 5,
            "noise should not read as a melody: {} notes",
            notes.len()
        );
    }

    #[test]
    fn a_frequency_maps_to_the_note_it_is_nearest() {
        assert_eq!(hz_to_note(440.0).0, 69);
        assert_eq!(hz_to_note(261.63).0, 60);
        let (pitch, cents) = hz_to_note(437.0);
        assert_eq!(pitch, 69);
        assert!(cents < -10.0, "437 Hz is flat of A: {cents} cents");
    }
}
