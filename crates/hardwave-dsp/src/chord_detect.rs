//! Heuristic chord detector — analyzes a mono audio buffer via FFT,
//! builds a 12-bin chroma (pitch class profile), and scores it against
//! the 24 major / minor chord templates. Deterministic, no ML.

use rustfft::{num_complex::Complex32, FftPlanner};

/// Quality of a detected chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordQuality {
    Major,
    Minor,
}

/// A detected chord: root pitch class (0 = C, 1 = C#, …, 11 = B) and
/// quality. `confidence` is in `[0, 1]` — the inner product of the
/// normalized chroma with the winning template.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectedChord {
    pub root: u8,
    pub quality: ChordQuality,
    pub confidence: f32,
}

impl DetectedChord {
    /// Human-readable name like "C", "F#m", "Bb", etc.
    pub fn name(&self) -> String {
        const NAMES: [&str; 12] = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        let base = NAMES[self.root as usize % 12];
        match self.quality {
            ChordQuality::Major => base.to_string(),
            ChordQuality::Minor => format!("{base}m"),
        }
    }
}

/// Detect the single most-likely major or minor triad in the buffer.
/// Returns `None` if the signal is effectively silent.
pub fn detect_chord(samples: &[f32], sample_rate: f32) -> Option<DetectedChord> {
    let chroma = compute_chroma(samples, sample_rate)?;
    let (root, quality, confidence) = best_template_match(&chroma);
    Some(DetectedChord {
        root,
        quality,
        confidence,
    })
}

/// Compute a 12-bin chroma vector from the input buffer. Returns
/// `None` when the signal's total energy is below the silence floor.
pub fn compute_chroma(samples: &[f32], sample_rate: f32) -> Option<[f32; 12]> {
    if samples.is_empty() || sample_rate <= 0.0 {
        return None;
    }
    // Use a power-of-two FFT length that matches the input (zero-padded
    // if needed). Cap at 16384 — chroma only needs coarse resolution.
    let n = samples.len().min(16_384).next_power_of_two().max(1024);
    let mut buffer: Vec<Complex32> = (0..n)
        .map(|i| {
            let x = samples.get(i).copied().unwrap_or(0.0);
            // Hann window reduces spectral leakage so tonal peaks land
            // cleanly in a single bin.
            let w = if n > 1 {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos())
            } else {
                1.0
            };
            Complex32::new(x * w, 0.0)
        })
        .collect();

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    fft.process(&mut buffer);

    let mut chroma = [0.0_f32; 12];
    let mut total_mag = 0.0_f32;
    // Ignore DC and the upper half of the spectrum (conjugate
    // mirror). Only consider bins whose centre frequency is in the
    // usable musical range 55 Hz – 2000 Hz (A1 to ~B6).
    let nyquist = sample_rate / 2.0;
    for (bin, c) in buffer.iter().take(n / 2).enumerate().skip(1) {
        let freq = bin as f32 * sample_rate / n as f32;
        if !(55.0..=2000.0).contains(&freq) || freq >= nyquist {
            continue;
        }
        let mag = c.norm();
        total_mag += mag;
        // MIDI note number of this frequency, then pitch class.
        let midi = 69.0 + 12.0 * (freq / 440.0).log2();
        let pc = ((midi.round() as i32).rem_euclid(12)) as usize;
        chroma[pc] += mag;
    }
    if total_mag < 1e-4 {
        return None;
    }
    // Normalize to unit L2.
    let norm = chroma.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut chroma {
            *v /= norm;
        }
    }
    Some(chroma)
}

/// Score `chroma` against all 24 major/minor triad templates and
/// return the winning `(root_pc, quality, confidence)`.
fn best_template_match(chroma: &[f32; 12]) -> (u8, ChordQuality, f32) {
    // Major triad: root, major 3rd (+4 st), perfect 5th (+7 st).
    // Minor triad: root, minor 3rd (+3 st), perfect 5th (+7 st).
    let major_tpl = build_template(&[0, 4, 7]);
    let minor_tpl = build_template(&[0, 3, 7]);

    let mut best_score = f32::MIN;
    let mut best_root = 0_u8;
    let mut best_q = ChordQuality::Major;
    for root in 0..12_u8 {
        let maj = dot_rotated(chroma, &major_tpl, root as usize);
        if maj > best_score {
            best_score = maj;
            best_root = root;
            best_q = ChordQuality::Major;
        }
        let min = dot_rotated(chroma, &minor_tpl, root as usize);
        if min > best_score {
            best_score = min;
            best_root = root;
            best_q = ChordQuality::Minor;
        }
    }
    (best_root, best_q, best_score.clamp(0.0, 1.0))
}

fn build_template(intervals: &[usize]) -> [f32; 12] {
    let mut t = [0.0_f32; 12];
    for &iv in intervals {
        t[iv % 12] = 1.0;
    }
    let norm = t.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut t {
            *v /= norm;
        }
    }
    t
}

fn dot_rotated(chroma: &[f32; 12], template: &[f32; 12], rotate_by: usize) -> f32 {
    let mut sum = 0.0_f32;
    for i in 0..12 {
        sum += chroma[(i + rotate_by) % 12] * template[i];
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin())
            .collect()
    }

    fn mix(parts: &[Vec<f32>]) -> Vec<f32> {
        let n = parts.iter().map(|p| p.len()).min().unwrap_or(0);
        (0..n).map(|i| parts.iter().map(|p| p[i]).sum()).collect()
    }

    #[test]
    fn chroma_peaks_on_a440() {
        let sr = 44_100.0;
        let buf = sine(440.0, sr, 8192);
        let chroma = compute_chroma(&buf, sr).expect("chroma");
        // Pitch class 9 = A. Should dominate the vector.
        let mut max_idx = 0;
        let mut max_val = f32::MIN;
        for (i, v) in chroma.iter().enumerate() {
            if *v > max_val {
                max_val = *v;
                max_idx = i;
            }
        }
        assert_eq!(max_idx, 9, "A440 should peak at pitch class 9 (A)");
    }

    #[test]
    fn detects_c_major_triad() {
        let sr = 44_100.0;
        // C4 = 261.63, E4 = 329.63, G4 = 392.00
        let buf = mix(&[
            sine(261.63, sr, 8192),
            sine(329.63, sr, 8192),
            sine(392.00, sr, 8192),
        ]);
        let chord = detect_chord(&buf, sr).expect("chord");
        assert_eq!(chord.root, 0, "root should be C (pc=0), got {}", chord.root);
        assert_eq!(chord.quality, ChordQuality::Major);
        assert_eq!(chord.name(), "C");
        assert!(
            chord.confidence > 0.8,
            "confidence should be high, got {}",
            chord.confidence
        );
    }

    #[test]
    fn detects_a_minor_triad() {
        let sr = 44_100.0;
        // A3 = 220, C4 = 261.63, E4 = 329.63
        let buf = mix(&[
            sine(220.0, sr, 8192),
            sine(261.63, sr, 8192),
            sine(329.63, sr, 8192),
        ]);
        let chord = detect_chord(&buf, sr).expect("chord");
        assert_eq!(chord.root, 9, "root should be A (pc=9), got {}", chord.root);
        assert_eq!(chord.quality, ChordQuality::Minor);
        assert_eq!(chord.name(), "Am");
    }

    #[test]
    fn silence_returns_none() {
        let sr = 44_100.0;
        let buf = vec![0.0_f32; 4096];
        assert!(detect_chord(&buf, sr).is_none());
    }

    #[test]
    fn chord_name_formatting() {
        let c = DetectedChord {
            root: 0,
            quality: ChordQuality::Major,
            confidence: 1.0,
        };
        assert_eq!(c.name(), "C");
        let fsharp_m = DetectedChord {
            root: 6,
            quality: ChordQuality::Minor,
            confidence: 1.0,
        };
        assert_eq!(fsharp_m.name(), "F#m");
    }
}
