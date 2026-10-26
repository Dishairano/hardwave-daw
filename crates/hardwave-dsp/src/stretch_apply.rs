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

/// One stretch of a warped clip: where it starts in the output, where in
/// the source, and how many source frames go by per output sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarpPiece {
    pub out_start: u64,
    pub src_start: f64,
    pub step: f64,
}

/// Source frames of context stretched with each piece and then cut away, so
/// the phase vocoder has settled by the piece's first sample.
const WARP_CONTEXT: usize = 4096;
/// Output samples either side of a join that the two pieces crossfade over.
const WARP_JOIN: usize = 128;

/// Render a warped clip with its pitch kept.
///
/// Warp markers used to play each stretch between two markers faster or
/// slower, like tape, so a phrase pulled into the grid also went up or
/// down in pitch. Here each piece of the source is time-stretched by the
/// phase vocoder to the length the markers give it (with context either
/// side, cut away after), shifted by `pitch_semitones`, and the pieces are
/// joined with short crossfades. `pieces` are in output order; the last
/// one runs to the end of the source. Output sample `n` is the clip's
/// `n`th sample on the timeline.
pub fn warp_render(
    input: &[f32],
    pieces: &[WarpPiece],
    pitch_semitones: f32,
    sample_rate: f32,
) -> Vec<f32> {
    let len = input.len();
    if len == 0 || pieces.is_empty() {
        return Vec::new();
    }
    // Where each piece ends in the output: the next one's start, or for the
    // last, where the source runs out at its pace.
    let ends: Vec<u64> = (0..pieces.len())
        .map(|i| match pieces.get(i + 1) {
            Some(next) => next.out_start,
            None => {
                let p = pieces[i];
                let left = (len as f64 - p.src_start).max(0.0);
                if p.step > 1e-9 {
                    p.out_start + (left / p.step).floor() as u64
                } else {
                    p.out_start
                }
            }
        })
        .collect();
    let out_len = ends.iter().copied().max().unwrap_or(0) as usize;
    let mut out = vec![0.0f32; out_len];
    for (i, piece) in pieces.iter().enumerate() {
        let (start, end) = (piece.out_start as usize, (ends[i] as usize).min(out_len));
        if end <= start || piece.step <= 1e-9 {
            continue;
        }
        let span = end - start;
        let src0 = piece.src_start.clamp(0.0, len as f64);
        let src1 = (src0 + span as f64 * piece.step).min(len as f64);
        if src1 - src0 < 1.0 {
            continue;
        }
        // The crossfade at each join, no longer than half the piece.
        let join = WARP_JOIN.min(span / 2);
        let a = (src0 as usize).saturating_sub(WARP_CONTEXT);
        let b = ((src1.ceil() as usize) + WARP_CONTEXT).min(len);
        let ratio = span as f64 / (src1 - src0);
        let stretched = apply_stretch(
            &input[a..b],
            StretchRequest {
                algorithm: StretchAlgorithm::PhaseVocoder,
                stretch_ratio: ratio as f32,
                pitch_semitones,
                sample_rate,
            },
        );
        if stretched.is_empty() {
            continue;
        }
        // Where the piece's source start landed in the stretched context.
        let scale = stretched.len() as f64 / (b - a) as f64;
        let origin = (src0 - a as f64) * scale;
        let at = |k: f64| -> f32 {
            let x = origin + k;
            if x < 0.0 {
                return 0.0;
            }
            let i0 = x.floor() as usize;
            let frac = (x - i0 as f64) as f32;
            let s0 = stretched.get(i0).copied().unwrap_or(0.0);
            let s1 = stretched.get(i0 + 1).copied().unwrap_or(s0);
            s0 + (s1 - s0) * frac
        };
        let first = i == 0;
        let last = i + 1 == pieces.len();
        let from = if first {
            start
        } else {
            start.saturating_sub(join)
        };
        let to = if last { end } else { (end + join).min(out_len) };
        for (n, slot) in out.iter_mut().enumerate().take(to).skip(from) {
            // Linear crossfades centred on each join: this piece fades in
            // across the join before it and out across the one after.
            let mut w = 1.0f32;
            if !first && join > 0 && n < start + join {
                w *= ((n + join) as f32 - start as f32) / (2 * join) as f32;
            }
            if !last && join > 0 && n + join >= end {
                w *= ((end + join) as f32 - n as f32) / (2 * join) as f32;
            }
            *slot += at((n as f64) - start as f64) * w.clamp(0.0, 1.0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn crossings(x: &[f32]) -> usize {
        x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    }

    /// Two pieces pulled to different lengths keep the note's pitch, where
    /// the old warp moved it with the speed, and meet without a jump.
    #[test]
    fn a_warped_clip_keeps_its_pitch_and_joins_smoothly() {
        let sr = 48_000.0;
        let input = sine(440.0, sr, 96_000);
        // The first second played over 1.5 s, the second over 0.75 s.
        let pieces = [
            WarpPiece {
                out_start: 0,
                src_start: 0.0,
                step: 48_000.0 / 72_000.0,
            },
            WarpPiece {
                out_start: 72_000,
                src_start: 48_000.0,
                step: 48_000.0 / 36_000.0,
            },
        ];
        let out = warp_render(&input, &pieces, 0.0, sr);
        assert_eq!(out.len(), 108_000, "1.5 s and 0.75 s");
        // Away from the edges, the pitch is still 440 Hz in both pieces.
        let hz = |from: usize, n: usize| crossings(&out[from..from + n]) as f32 * sr / n as f32;
        let first = hz(10_000, 48_000);
        let second = hz(78_000, 24_000);
        assert!((first - 440.0).abs() < 8.0, "first piece at {first} Hz");
        assert!((second - 440.0).abs() < 8.0, "second piece at {second} Hz");
        // No click at the join: no step bigger than a sine's own.
        let max_step = out[71_000..73_000]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        let sine_step = 2.0 * PI * 440.0 / sr;
        assert!(
            max_step < sine_step * 1.6,
            "a jump of {max_step} at the join"
        );
    }

    #[test]
    fn a_warp_with_a_shift_moves_the_pitch_only() {
        let sr = 48_000.0;
        let input = sine(440.0, sr, 48_000);
        let pieces = [WarpPiece {
            out_start: 0,
            src_start: 0.0,
            step: 1.0,
        }];
        let out = warp_render(&input, &pieces, 12.0, sr);
        assert_eq!(out.len(), 48_000);
        let hz = crossings(&out[8_000..40_000]) as f32 * sr / 32_000.0;
        assert!((hz - 880.0).abs() < 15.0, "an octave up: {hz} Hz");
    }

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
