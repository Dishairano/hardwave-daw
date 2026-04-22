//! Destructive time-stretch / pitch-shift — glue between the three
//! stretch algorithms (`time_pitch` OLA, `phase_vocoder`, `granular`)
//! and the clip-level "commit to buffer" operation that the clip UI
//! triggers when the user chooses "Apply stretch destructively".

use crate::{granular, phase_vocoder, time_pitch};

/// Which stretch algorithm to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchAlgorithm {
    /// Resample only — classic speed / pitch coupled.
    Resample,
    /// OLA time stretch + resample for pitch shift.
    OverlapAdd,
    /// Phase vocoder — higher quality for tonal sources.
    PhaseVocoder,
    /// Granular — robust at extreme ratios.
    Granular,
}

/// Request parameters for a destructive stretch.
#[derive(Debug, Clone, Copy)]
pub struct StretchRequest {
    pub algorithm: StretchAlgorithm,
    pub stretch_ratio: f32,
    pub pitch_semitones: f32,
    pub sample_rate: f32,
}

impl StretchRequest {
    pub fn identity(sample_rate: f32) -> Self {
        Self {
            algorithm: StretchAlgorithm::OverlapAdd,
            stretch_ratio: 1.0,
            pitch_semitones: 0.0,
            sample_rate,
        }
    }

    pub fn is_identity(&self) -> bool {
        (self.stretch_ratio - 1.0).abs() < 1e-4 && self.pitch_semitones.abs() < 1e-4
    }
}

/// Apply the request to `input` and return a new buffer. Identity
/// requests short-circuit to a clone, so the call is cheap when the
/// UI pops the dialog and the user accepts defaults.
pub fn apply_stretch(input: &[f32], req: StretchRequest) -> Vec<f32> {
    if input.is_empty() || req.is_identity() {
        return input.to_vec();
    }
    // 1. Stretch time by ratio (keep pitch), algorithm-specific.
    let stretched = match req.algorithm {
        StretchAlgorithm::Resample => {
            // Resample changes both duration and pitch by the same ratio.
            // Achieve pure time stretch by resample + invert pitch (via OLA).
            // Simpler: treat Resample as "no pitch preservation" → resample.
            let out_len = ((input.len() as f32) * req.stretch_ratio) as usize;
            time_pitch::resample_to_length(input, out_len)
        }
        StretchAlgorithm::OverlapAdd => {
            time_pitch::time_stretch_ola(input, req.stretch_ratio, 1024, 256)
        }
        StretchAlgorithm::PhaseVocoder => {
            phase_vocoder::phase_vocoder_stretch(input, req.stretch_ratio, 2048)
        }
        StretchAlgorithm::Granular => {
            granular::granular_stretch(input, req.sample_rate, req.stretch_ratio, 40.0, 0.5)
        }
    };
    // 2. Pitch shift the stretched buffer without changing its
    //    length. The pitch shift path chooses the same-family
    //    algorithm.
    if req.pitch_semitones.abs() < 1e-4 {
        return stretched;
    }
    match req.algorithm {
        StretchAlgorithm::Resample => {
            // Resample already shifted the pitch by the same ratio;
            // additional semitone-based shift via linear resample.
            let pitch_ratio = 2_f32.powf(req.pitch_semitones / 12.0);
            let out_len = ((stretched.len() as f32) / pitch_ratio) as usize;
            time_pitch::resample_to_length(&stretched, out_len)
        }
        StretchAlgorithm::OverlapAdd => time_pitch::pitch_shift(&stretched, req.pitch_semitones),
        StretchAlgorithm::PhaseVocoder => {
            phase_vocoder::pitch_shift_pv(&stretched, req.pitch_semitones)
        }
        StretchAlgorithm::Granular => {
            granular::pitch_shift_granular(&stretched, req.sample_rate, req.pitch_semitones)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine(freq: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sr).sin())
            .collect()
    }

    #[test]
    fn identity_request_returns_clone() {
        let input = sine(440.0, 44_100.0, 1024);
        let out = apply_stretch(&input, StretchRequest::identity(44_100.0));
        assert_eq!(out.len(), input.len());
        assert_eq!(out, input);
    }

    #[test]
    fn empty_input_returns_empty() {
        let r = StretchRequest {
            algorithm: StretchAlgorithm::PhaseVocoder,
            stretch_ratio: 2.0,
            pitch_semitones: 5.0,
            sample_rate: 48_000.0,
        };
        let out = apply_stretch(&[], r);
        assert!(out.is_empty());
    }

    #[test]
    fn ola_ratio_2_roughly_doubles_length() {
        let sr = 44_100.0;
        let input = sine(440.0, sr, 8192);
        let r = StretchRequest {
            algorithm: StretchAlgorithm::OverlapAdd,
            stretch_ratio: 2.0,
            pitch_semitones: 0.0,
            sample_rate: sr,
        };
        let out = apply_stretch(&input, r);
        let diff = (out.len() as i32 - (input.len() * 2) as i32).abs();
        assert!(diff < 2048);
    }

    #[test]
    fn phase_vocoder_ratio_and_pitch_combined() {
        let sr = 44_100.0;
        let input = sine(440.0, sr, 8192);
        let r = StretchRequest {
            algorithm: StretchAlgorithm::PhaseVocoder,
            stretch_ratio: 1.5,
            pitch_semitones: 7.0,
            sample_rate: sr,
        };
        let out = apply_stretch(&input, r);
        let expected_len = (input.len() as f32 * 1.5) as usize;
        let diff = (out.len() as i32 - expected_len as i32).abs();
        assert!(diff < 4096, "len {} expected {}", out.len(), expected_len);
    }

    #[test]
    fn all_algorithms_produce_nonzero_output_on_sine() {
        let sr = 44_100.0;
        let input = sine(440.0, sr, 8192);
        for algo in [
            StretchAlgorithm::Resample,
            StretchAlgorithm::OverlapAdd,
            StretchAlgorithm::PhaseVocoder,
            StretchAlgorithm::Granular,
        ] {
            let r = StretchRequest {
                algorithm: algo,
                stretch_ratio: 1.25,
                pitch_semitones: 3.0,
                sample_rate: sr,
            };
            let out = apply_stretch(&input, r);
            let peak = out.iter().fold(0.0_f32, |acc, v| acc.max(v.abs()));
            assert!(peak > 0.1, "algo {:?} silent, peak {}", algo, peak);
        }
    }
}
